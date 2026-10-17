//! Driving one scanner through a capture.

use crate::{
    eeprom::{RegisterTable, parse_tables},
    protocol::{
        SEL_BRIDGE_REG, SEL_CONTROL, SEL_EEPROM, SEL_SENSOR_REG, control_payload, eeprom,
        eeprom_payload, key, sensor_payload,
    },
    steady,
};
use passport_scanner_core::{
    DocumentPresence, Error, Frame, FrameSource, LightSource, Result, VendorTransport,
};
use std::{thread::sleep, time::Duration};

/// How long to let the image settle after changing illumination.
///
/// The hardware gives no ready signal. The shipped driver waits a fixed delay
/// taken from its configuration - `DelayTime-5M-<source>`, 400 ms for both
/// lamps on the reference unit - and then takes the next frame the stream
/// delivers. That figure is now read from its configuration rather than
/// guessed at; see [`crate::settings`]. This is the fallback for a machine
/// that has no vendor install to read.
///
/// [`crate::steady`] recognises the new picture rather than relying on the
/// wait, so a shorter one does not produce a wrong image - swept over real
/// scans, 400, 100, 50 and 0 ms all returned the same tonal spread. The lamp
/// does take time to reach its working output, but a frame taken while it
/// is still rising differs from the next one, and the steady check keeps
/// taking frames until two agree. A short wait is kept as a margin; see
/// [`crate::settings::LAMP_WAIT_CAP`].
pub const DEFAULT_SETTLE: Duration = crate::settings::LAMP_WAIT_CAP;

/// Value that frees the imaging path. Without it the stream yields nothing at
/// all - two-byte frames - no matter which lamp is lit.
pub const ENABLE: u8 = 0x01;
/// Value that shuts the imaging path again.
pub const DISABLE: u8 = 0x00;

/// How long the buzzer sounds when a scan completes.
pub const BUZZER: Duration = Duration::from_millis(300);

/// How many EEPROM bytes to take from one exchange.
///
/// A reply carries the opcode in byte 0 and the contents from the requested
/// address in the rest, so a 35-byte buffer yields 34. Reading a block eight
/// bytes at a time, as an earlier pass did out of caution, turned the
/// calibration read into seventy round trips for no reason.
pub const EEPROM_CHUNK: usize = 32;

/// Frames to throw away when the stream first comes up.
///
/// The first frames out of a cold stream are not of anything - the sensor is
/// still coming to a working exposure. Within a scan, frames are not counted
/// but recognised; see [`crate::steady`].
pub const FLUSH_FRAMES: usize = 2;

/// Factory white-light settings and the shading file measured with them.
/// General remains the production default; Identity is an explicit alternative.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CaptureProfile {
    #[default]
    General,
    Identity,
}

impl CaptureProfile {
    /// The vendor shading file that must accompany this profile's white frame.
    #[must_use]
    pub const fn shading_file(self) -> &'static str {
        match self {
            Self::General => "Calibration_General.shd",
            Self::Identity => "Calibration_General_ID.shd",
        }
    }
}

/// The sensor register tables this unit carries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Calibration {
    /// Registers that put the sensor into its working mode, applied once.
    pub common: Vec<crate::eeprom::RegisterWrite>,
    /// Per-unit AWB for the selected white-light profile.
    pub white_balance: Vec<crate::eeprom::RegisterWrite>,
    /// General AWB used for infrared regardless of the selected white profile.
    pub infrared_white_balance: Vec<crate::eeprom::RegisterWrite>,
    /// Selected white-light table (General A1 or Identity A3).
    pub white: Vec<crate::eeprom::RegisterWrite>,
    /// Registers to load before an infrared capture.
    pub infrared: Vec<crate::eeprom::RegisterWrite>,
    /// Selected white exposure trim (General 3FFC or Identity 3FEC).
    pub white_exposure: Option<crate::eeprom::RegisterWrite>,
    /// Per-unit gain trim for infrared, from `0x3FF8`.
    pub infrared_gain: Option<crate::eeprom::RegisterWrite>,
    /// The unit's calibration stamp, which ties stored shading to this device.
    pub stamp: u64,
}

/// A scanner, driven through a vendor channel and a frame source.
#[derive(Debug)]
pub struct Scanner<T, C> {
    transport: T,
    capture: C,
    calibration: Calibration,
    profile: CaptureProfile,
    /// Explicit diagnostic F1 override, never used by the default profiles.
    white_lamp_override: Option<bool>,
    /// Tracks ID illumination so IR, General, and cleanup clear only its bit.
    identity_lamp_active: bool,
    /// Vendor IDLight flag requests a clear even without a local ID history.
    clear_inherited_id_lamp: bool,
    settle: Duration,
    /// Image controls to apply before a white capture.
    white_controls: Vec<(String, i64)>,
    /// Restore known native defaults before writing the sensor tables.
    startup_controls: Vec<(String, i64)>,
    /// Whether to load the per-source register tables before a capture.
    source_tables: bool,
    /// Whether to sound the buzzer when a scan completes.
    buzzer: bool,
    /// How many frames to throw away when the stream first comes up.
    flush: usize,
    /// Size of the last frame kept, for telling a new picture from a stale one.
    last_size: Option<usize>,
}

impl<T: VendorTransport, C: FrameSource> Scanner<T, C> {
    /// Take ownership of a transport and a capture backend.
    pub fn new(transport: T, capture: C) -> Self {
        Self {
            transport,
            capture,
            calibration: Calibration::default(),
            profile: CaptureProfile::General,
            white_lamp_override: None,
            identity_lamp_active: false,
            clear_inherited_id_lamp: false,
            startup_controls: Vec::new(),
            settle: DEFAULT_SETTLE,
            white_controls: Vec::new(),
            source_tables: true,
            // Off by default: it is three tenths of a second of waiting on every
            // scan, and an operator watching the screen does not need it.
            buzzer: false,
            flush: FLUSH_FRAMES,
            last_size: None,
        }
    }

    /// Replace the image controls applied before a white capture.
    #[must_use]
    pub fn with_white_controls(mut self, controls: Vec<(String, i64)>) -> Self {
        self.white_controls = controls;
        self
    }

    /// Change how many frames to throw away when the stream comes up.
    pub fn set_flush(&mut self, frames: usize) {
        self.flush = frames;
    }

    /// Change how long to wait after switching illumination.
    pub fn set_settle(&mut self, settle: Duration) {
        self.settle = settle;
    }

    /// Take timings and image controls from the shipped driver's own
    /// configuration for this model.
    ///
    /// Everything it carries wins over the defaults here, because the defaults
    /// are this code's guesses and that file is what the device was built
    /// around. A control the device does not report, or a value outside the
    /// range it reports, is skipped rather than forced: the shipped
    /// application works in its own scale and converts somewhere this code
    /// cannot see, so a value that does not fit is one whose meaning is not
    /// established.
    pub fn apply_settings(&mut self, settings: &crate::settings::Settings) {
        self.settle = settings.delays.white;
        self.clear_inherited_id_lamp = settings.id_light;
        // Not the buzzer. The file says to sound it and the device will, but
        // whether a scanner beeps at the person using it is their call and not
        // the vendor's; [`Scanner::set_buzzer`] is how it gets turned on.

        // CmdEGW selects the sensor-command branch in the shipped driver.
        // CameraProp numbers belong to its alternative generic camera path;
        // applying them here can overwrite the sensor tables with unrelated
        // UVC controls. Also clear overrides left by an earlier settings load.
        self.white_controls.clear();
        self.startup_controls.clear();
        if settings.sensor_controls {
            self.startup_controls = self.capture.control_defaults();
            return;
        }
        for name in settings.white.automatic_controls() {
            tracing::warn!(
                control = name,
                "automatic camera mode is unsupported; leaving control unchanged"
            );
        }
        let available = self.capture.controls();
        self.white_controls = settings
            .white
            .controls()
            .into_iter()
            .filter_map(|(name, value)| {
                let control = available.iter().find(|c| c.name == name)?;
                control.accepts(value).then(|| (control.name.clone(), value))
            })
            .collect();
    }

    /// Sound the buzzer when a scan completes, or do not.
    pub fn set_buzzer(&mut self, enabled: bool) {
        self.buzzer = enabled;
    }

    /// Load the per-source register tables before a capture, or do not.
    ///
    /// They set the sensor metering window, lens shading and exposure, and are
    /// what the shipped driver loads. Turning them off is for finding out what
    /// they actually contribute.
    pub fn set_source_tables(&mut self, enabled: bool) {
        self.source_tables = enabled;
    }

    /// Change the image controls applied before a white capture.
    pub fn set_white_controls(&mut self, controls: Vec<(String, i64)>) {
        self.white_controls = controls;
    }

    /// Override the volatile white exposure trim for a diagnostic capture.
    /// This does not write EEPROM or change the factory calibration on disk.
    pub fn set_white_exposure(&mut self, value: u8) {
        self.calibration.white_exposure =
            Some(crate::eeprom::RegisterWrite { register: 0x3501, value: u16::from(value) });
    }

    /// Override the settling delay.
    #[must_use]
    pub fn with_settle(mut self, settle: Duration) -> Self {
        self.settle = settle;
        self
    }

    /// What was read out of the unit's EEPROM.
    #[must_use]
    pub fn calibration(&self) -> &Calibration {
        &self.calibration
    }

    /// Select a complete factory profile, reading its settings from EEPROM.
    /// The caller must pair white frames with [`CaptureProfile::shading_file`].
    /// No EEPROM or sensor registers are written by this operation.
    ///
    /// # Errors
    /// Leaves the previous profile intact if reading or validating fails.
    pub fn set_capture_profile(&mut self, profile: CaptureProfile) -> Result<()> {
        self.set_capture_profile_with_exposure_percent(profile, 100)
    }

    /// Select factory settings and scale their white exposure as one operation.
    /// A percentage of 100 keeps the factory value; each selection starts from
    /// EEPROM again, so repeated selections do not compound the adjustment.
    /// The rounded result must fit the sensor's nonzero 8-bit exposure register.
    /// No sensor or EEPROM writes are performed here.
    ///
    /// # Errors
    /// Invalid percentages, absent/invalid trims, out-of-range results, or EEPROM
    /// read errors leave both the selected profile and calibration unchanged.
    pub fn set_capture_profile_with_exposure_percent(
        &mut self,
        profile: CaptureProfile,
        percent: u16,
    ) -> Result<()> {
        if percent == 0 {
            return Err(Error::Rejected("exposure percentage must be greater than zero".into()));
        }
        let mut calibration = self.read_calibration_for(profile)?;
        let trim = calibration
            .white_exposure
            .as_mut()
            .ok_or_else(|| Error::Malformed("factory profile has no white exposure trim".into()))?;
        if trim.register != 0x3501 || !(1..=255).contains(&trim.value) {
            return Err(Error::Malformed(format!(
                "factory white exposure must target 3501 with a value from 1 through 255, got {:04X}={:04X}",
                trim.register, trim.value
            )));
        }
        let scaled = (u32::from(trim.value) * u32::from(percent) + 50) / 100;
        if !(1..=255).contains(&scaled) {
            return Err(Error::Rejected(format!(
                "exposure at {percent}% rounds to {scaled}, outside 1 through 255"
            )));
        }
        trim.value = scaled as u16;
        self.calibration = calibration;
        self.profile = profile;
        Ok(())
    }

    #[must_use]
    pub fn capture_profile(&self) -> CaptureProfile {
        self.profile
    }

    /// Override the F1 lamp switch for white captures only, for diagnostics.
    /// Identity plus `Some(true)` is not a claim of vendor-matching illumination.
    /// A diagnostic override suppresses native ID illumination to isolate F1.
    /// `None` restores the selected profile's behavior; EEPROM is untouched.
    pub fn set_diagnostic_white_lamp(&mut self, enabled: Option<bool>) {
        self.white_lamp_override = enabled;
    }

    /// Bring the unit into a known state and load its calibration.
    ///
    /// # Errors
    /// Fails if the unit does not answer, or its EEPROM cannot be read.
    pub fn initialise(&mut self) -> Result<()> {
        self.control(key::IMAGING_ENABLE, DISABLE)?;
        self.control(key::INIT_STEP, 0x01)?;

        self.calibration = self.read_calibration()?;

        // The common table takes the sensor out of whatever mode it powered up
        // in - manual exposure and gain, fixed white balance, ISP configured.
        // Without it the per-source tables land on a sensor that ignores them.
        let common = self.calibration.common.clone();
        self.write_registers(&common)?;
        Ok(())
    }

    /// Is a document on the platen?
    ///
    /// # Errors
    /// Fails if the unit does not answer.
    pub fn document_present(&mut self) -> Result<DocumentPresence> {
        let a = self.read_key(key::SENSOR_A)?;
        let b = self.read_key(key::SENSOR_B)?;
        Ok(DocumentPresence::from_sensors(a & 1 == 1, b & 1 == 1))
    }

    /// Capture one frame under `source`.
    ///
    /// Loads that source's register table, switches illumination, waits out the
    /// settling delay, and takes the next frame.
    ///
    /// # Errors
    /// Fails if the unit refuses a command or no frame arrives.
    pub fn capture(&mut self, source: LightSource) -> Result<Frame> {
        let table = match source {
            LightSource::White => self.calibration.white.clone(),
            LightSource::Infrared => self.calibration.infrared.clone(),
            LightSource::Ultraviolet => {
                return Err(Error::unsupported("this unit has no ultraviolet lamp"));
            }
        };

        // One lamp at a time: the other goes out first so the two never
        // overlap in a frame.
        let infrared = source == LightSource::Infrared;
        // Identity clears F1/F2, then sets bridge register 7 bit 0x40.
        // General/IR clear that bit when leaving an active Identity source.
        let identity = !infrared
            && self.profile == CaptureProfile::Identity
            && self.white_lamp_override.is_none();
        if !identity && (self.identity_lamp_active || (!infrared && self.clear_inherited_id_lamp)) {
            self.set_identity_lamp(false)?;
        }
        let general_white = !infrared
            && self.white_lamp_override.unwrap_or(self.profile == CaptureProfile::General);
        self.control(key::WHITE_LAMP, u8::from(general_white))?;
        self.control(key::IR_LAMP, u8::from(infrared))?;
        if identity {
            self.set_identity_lamp(true)?;
        }

        if self.source_tables {
            let white_balance = if infrared {
                self.calibration.infrared_white_balance.clone()
            } else {
                self.calibration.white_balance.clone()
            };
            self.write_registers(&white_balance)?;
            self.write_registers(&table)?;
        }

        // The per-unit trim from EEPROM goes on last, exactly as the shipped
        // driver applies it - it overrides the value the table just set.
        let trim = match source {
            LightSource::White => self.calibration.white_exposure,
            LightSource::Infrared => self.calibration.infrared_gain,
            LightSource::Ultraviolet => None,
        };
        if self.source_tables
            && let Some(write) = trim
        {
            self.write_registers(&[write])?;
        }

        // The image controls go on last, after every register write, so the
        // capture stack is not handed a brighter setting afterwards. A source
        // without controls is fine - the picture is then whatever the stack
        // chose - so this must not fail a capture.
        if source == LightSource::White {
            for (name, value) in self.white_controls.clone() {
                let _ = self.capture.set_control(&name, value);
            }
        }

        sleep(self.settle);

        // Take frames until one is both the new picture and no longer moving.
        // Counting a fixed number of them is the obvious alternative and it is
        // what this used to do, but measured over nine scans two was sometimes
        // too few and four was sometimes no better: how far behind the stream
        // runs is not constant, so it has to be asked rather than assumed.
        // Clear whatever the stream was holding before looking at content at
        // all. A frame that comes back instantly was exposed before any of
        // the changes above; one that takes a frame interval was not.
        let mut frame = None;
        for _ in 0..steady::DRAIN {
            let started = std::time::Instant::now();
            let taken = self.capture.next_frame(Duration::from_secs(5))?;
            let instant = steady::was_buffered(started.elapsed());
            frame = Some(taken);
            if !instant {
                break;
            }
        }
        // DRAIN is a ceiling, not a target: if the stream will not stop
        // handing back buffered frames, the last one is still a frame.
        let mut frame = frame.ok_or_else(|| Error::Rejected("no frame arrived".to_owned()))?;

        let before = self.last_size;
        let mut previous = None;
        for _ in 1..steady::LIMIT {
            if steady::is_settled(frame.data.len(), previous, before) {
                break;
            }
            previous = Some(frame.data.len());
            frame = self.capture.next_frame(Duration::from_secs(5))?;
        }

        self.last_size = Some(frame.data.len());
        frame.source = source;
        Ok(frame)
    }

    /// Light the platen, take one frame under `source`, and put the lamps out.
    ///
    /// The whole-scan path keeps the stream up across several sources; this is
    /// for taking a single frame on its own, such as when trying a setting.
    ///
    /// # Errors
    /// Fails if the unit refuses a command or no frame arrives.
    pub fn capture_once(&mut self, source: LightSource) -> Result<Frame> {
        let mut frames = self.scan(&[source])?;
        frames.pop().ok_or_else(|| Error::Rejected("no frame arrived".to_owned()))
    }

    /// Run a full capture: every fitted source, in order, then tidy up.
    ///
    /// # Errors
    /// Fails if any step does. Illumination is switched off even then.
    pub fn scan(&mut self, sources: &[LightSource]) -> Result<Vec<Frame>> {
        // F6 enables the imaging path; it does not select an illuminator.
        // After startup frames are discarded, capture() selects General
        // white with F1, infrared with F2, or Identity with bridge register 7.
        // Those lamp changes and sensor writes require a running pipeline.
        // A freshly opened capture handle, every scan. What that is for is
        // in FrameSource::reopen; in short, the first capture on a handle is
        // exposed correctly and the ones after it are not. Not fatal if the
        // stack cannot do it - a source with nothing to restore says so by
        // doing nothing.
        let mut frames = Vec::with_capacity(sources.len());
        let started = std::time::Instant::now();
        let mut lap = started;
        // Where a scan's time goes, for `RUST_LOG=debug`; nothing personal.
        let mut phase = |name: &str| {
            let now = std::time::Instant::now();
            tracing::debug!(phase = name, elapsed_ms = (now - lap).as_millis(), "scan phase");
            lap = now;
        };
        let outcome: Result<()> = (|| {
            self.capture.reopen()?;
            phase("reopen");
            self.control(key::IMAGING_ENABLE, ENABLE)?;
            self.capture.start()?;
            phase("start stream");
            self.last_size = None;
            for _ in 0..self.flush {
                let stale = self.capture.next_frame(Duration::from_secs(5))?;
                self.last_size = Some(stale.data.len());
            }
            phase("flush");
            // Opening the stream resets the sensor. Wait for startup frames
            // before restoring its manual exposure and white-balance mode.
            for (name, value) in self.startup_controls.clone() {
                self.capture.set_control(&name, value)?;
            }
            let common = self.calibration.common.clone();
            self.write_registers(&common)?;
            phase("restore controls");
            for source in sources {
                frames.push(self.capture(*source)?);
                phase(match source {
                    LightSource::White => "white",
                    LightSource::Infrared => "infrared",
                    _ => "other source",
                });
            }
            Ok(())
        })();

        // Tidy up whatever happened above, the stream included.
        //
        // Stopping it matters more than it looks. Left running between scans,
        // the first scan after opening the device came out right and every one
        // after it came out overexposed - measured over ten runs, the first
        // had 57 per cent of the frame dark and the other nine had 15 per
        // cent, identically. Closing and reopening the whole device between
        // runs fixed it; reinitialising only the device side did not. So what
        // the scan depends on is a freshly started stream, and the way to get
        // one is to stop the stream it started.
        // Bridge readback must happen while the stream is still live.
        let identity_cleanup =
            if self.identity_lamp_active { self.set_identity_lamp(false) } else { Ok(()) };
        let stop = self.capture.stop();
        let disable_imaging = self.control(key::IMAGING_ENABLE, DISABLE);
        let lamps_off = self.lights_off();
        // Attempt every cleanup action, then report the earliest failure.
        // A captured frame is not a successful scan if shutdown failed.
        tracing::debug!(elapsed_ms = started.elapsed().as_millis(), "scan captured");
        outcome?;
        identity_cleanup?;
        stop?;
        disable_imaging?;
        lamps_off?;

        if self.buzzer {
            self.buzz(BUZZER)?;
        }
        Ok(frames)
    }

    /// Turn every lamp off.
    ///
    /// # Errors
    /// Fails if the unit refuses a command.
    pub fn lights_off(&mut self) -> Result<()> {
        let identity =
            if self.identity_lamp_active { self.set_identity_lamp(false) } else { Ok(()) };
        let white = self.control(key::WHITE_LAMP, key::OFF);
        let infrared = self.control(key::IR_LAMP, key::OFF);
        identity?;
        white?;
        infrared
    }

    /// Sound the buzzer for `duration`.
    ///
    /// # Errors
    /// Fails if the unit refuses a command.
    pub fn buzz(&mut self, duration: Duration) -> Result<()> {
        self.control(key::BUZZER, key::ON)?;
        sleep(duration);
        self.control(key::BUZZER, key::OFF)
    }

    /// Read `len` bytes of EEPROM from `address`.
    ///
    /// One exchange returns a run of bytes, so a block needs far fewer requests
    /// than the shipped driver's byte-at-a-time loop.
    ///
    /// # Errors
    /// Fails if the unit refuses a read.
    pub fn read_eeprom(&mut self, address: u16, len: usize) -> Result<Vec<u8>> {
        let width = self.transport.payload_len();
        // byte 0 of the reply echoes the opcode, so a reply carries width - 1 bytes
        let per_read = EEPROM_CHUNK.min(width.saturating_sub(1));
        if per_read == 0 {
            return Err(Error::unsupported("the transport has no room for an EEPROM reply"));
        }

        let mut out = Vec::with_capacity(len);
        while out.len() < len {
            let at = address + out.len() as u16;
            let mut buf = eeprom_payload(at, width);
            self.transport.get(SEL_EEPROM, &mut buf)?;
            let take = per_read.min(len - out.len());
            out.extend_from_slice(&buf[1..=take]);
        }
        Ok(out)
    }

    fn read_calibration(&mut self) -> Result<Calibration> {
        self.read_calibration_for(self.profile)
    }

    fn read_calibration_for(&mut self, profile: CaptureProfile) -> Result<Calibration> {
        let hi = self.read_word(eeprom::STAMP_HI)?;
        let lo = self.read_word(eeprom::STAMP_LO)?;

        // The register tables sit in a block after a short header.
        let block = self.read_eeprom(eeprom::TABLES_BASE + 0x0D, 0x240)?;
        let tables = parse_tables(&block)?;

        // The three short tables are General white (A1), infrared (A2), and
        // ID-light white (A3). Table, AWB, trim and shading must use the same
        // selected profile. Infrared always uses its General-path settings.
        // See docs/implementation/PASSPORT-SCANNER.md (EEPROM section) for the vendor's actual loader.
        let short: Vec<&RegisterTable> = tables.iter().filter(|t| t.writes.len() == 29).collect();
        if short.len() != 3 {
            return Err(Error::Malformed(format!(
                "expected General, infrared and ID sensor tables, found {}",
                short.len()
            )));
        }
        let white_index = match profile {
            CaptureProfile::General => 0,
            CaptureProfile::Identity => 2,
        };
        let white = short.get(white_index).map(|t| t.writes.clone());
        let infrared = short.get(1).map(|t| t.writes.clone());

        let common = tables.first().map(|t| t.writes.clone()).unwrap_or_default();
        let infrared_white_balance = self.read_awb(eeprom::GENERAL_AWB)?;
        let (white_balance, exposure_address) = match profile {
            CaptureProfile::General => (infrared_white_balance.clone(), eeprom::WHITE_EXPOSURE),
            CaptureProfile::Identity => (self.read_awb(eeprom::ID_AWB)?, eeprom::ID_EXPOSURE),
        };

        Ok(Calibration {
            common,
            white_balance,
            infrared_white_balance,
            white: white.unwrap_or_default(),
            infrared: infrared.unwrap_or_default(),
            white_exposure: self.read_trim(exposure_address)?,
            infrared_gain: self.read_trim(eeprom::IR_GAIN)?,
            stamp: crate::calibration_stamp(hi, lo),
        })
    }

    fn read_awb(&mut self, addresses: [u16; 6]) -> Result<Vec<crate::eeprom::RegisterWrite>> {
        let mut white_balance = Vec::new();
        for (index, address) in addresses.into_iter().enumerate() {
            if let Some(write) = self.read_trim(address)? {
                let expected = 0x3400 + index as u16;
                if write.register != expected {
                    return Err(Error::Malformed(format!(
                        "AWB at {address:#06x} targets {:#06x}, expected {expected:#06x}",
                        write.register
                    )));
                }
                white_balance.push(write);
            }
        }

        Ok(white_balance)
    }

    /// Read one stored register/value pair, as the EEPROM keeps per-unit trim.
    fn read_trim(&mut self, address: u16) -> Result<Option<crate::eeprom::RegisterWrite>> {
        let bytes = self.read_eeprom(address, 4)?;
        let w = crate::eeprom::RegisterWrite::from_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        Ok((w.register != 0 && w.register != 0xFFFF).then_some(w))
    }

    fn read_word(&mut self, address: u16) -> Result<u32> {
        let bytes = self.read_eeprom(address, 4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Read a bridge register through opcode 1; values occupy reply bytes 1/2.
    /// This is distinct from sensor-register selector 0.
    ///
    /// # Errors
    /// Fails if the transport cannot read the register.
    pub fn read_bridge_register(&mut self, register: u16) -> Result<u16> {
        let mut payload = sensor_payload(register, 0, self.transport.payload_len());
        if payload.len() < 3 {
            return Err(Error::Malformed("bridge reply needs opcode and value".into()));
        }
        self.transport.get(SEL_BRIDGE_REG, &mut payload)?;
        Ok(u16::from_le_bytes([payload[1], payload[2]]))
    }

    fn set_identity_lamp(&mut self, enabled: bool) -> Result<()> {
        let before = self.read_bridge_register(7)?;
        // The vendor treats bridge register 7 as a byte and uses mask BF.
        let after = if enabled { before | 0x40 } else { before & 0xBF };
        let payload = sensor_payload(7, after, self.transport.payload_len());
        // If enabling fails after a partial transfer, cleanup must still try off.
        if enabled {
            self.identity_lamp_active = true;
        }
        self.transport.set(SEL_BRIDGE_REG, &payload)?;
        self.identity_lamp_active = enabled;
        Ok(())
    }

    /// Push a run of register writes at the sensor.
    fn write_registers(&mut self, writes: &[crate::eeprom::RegisterWrite]) -> Result<()> {
        let width = self.transport.payload_len();
        for w in writes {
            // A zero register is a gap in the stored table, not an address.
            if w.register == 0 {
                continue;
            }
            let payload = sensor_payload(w.register, w.value, width);
            self.transport.set(SEL_SENSOR_REG, &payload)?;
        }
        Ok(())
    }

    fn control(&mut self, k: u8, value: u8) -> Result<()> {
        let payload = control_payload(k, value, self.transport.payload_len());
        self.transport.set(SEL_CONTROL, &payload)
    }

    /// Read one control key and return its value byte.
    fn read_key(&mut self, k: u8) -> Result<u8> {
        let mut buf = control_payload(k, 0, self.transport.payload_len());
        self.transport.get(SEL_CONTROL, &mut buf)?;
        Ok(buf[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use passport_scanner_core::{PixelFormat, Selector};

    /// A transport that answers from a script, so the sequence can be checked
    /// without hardware.
    #[derive(Debug, Default)]
    struct Fake {
        writes: Vec<(u8, Vec<u8>)>,
        sensor_a: u8,
        sensor_b: u8,
        bridge_register_7: u16,
        fail_bridge_write: bool,
        fail_control_off: Option<u8>,
        eeprom: Vec<u8>,
    }

    impl VendorTransport for Fake {
        fn set(&mut self, selector: Selector, payload: &[u8]) -> Result<()> {
            self.writes.push((selector.get(), payload.to_vec()));
            if selector == SEL_BRIDGE_REG && self.fail_bridge_write {
                return Err(Error::Rejected("test bridge write failed".into()));
            }
            if selector == SEL_CONTROL
                && self.fail_control_off == Some(payload[0])
                && payload[1] == 0
            {
                return Err(Error::Rejected(format!("test control {:02X} off failed", payload[0])));
            }
            if selector == SEL_BRIDGE_REG && payload[..2] == [0, 7] {
                self.bridge_register_7 = u16::from_be_bytes([payload[2], payload[3]]);
            }
            Ok(())
        }
        fn get(&mut self, selector: Selector, buffer: &mut [u8]) -> Result<()> {
            if selector == SEL_BRIDGE_REG {
                assert_eq!(&buffer[..2], &[0, 7]);
                buffer[0] = 1;
                buffer[1..3].copy_from_slice(&self.bridge_register_7.to_le_bytes());
            }
            if selector == SEL_CONTROL {
                buffer[1] = match buffer[0] {
                    key::SENSOR_A => self.sensor_a,
                    key::IR_LAMP => self.sensor_b,
                    _ => 0,
                };
            }
            if selector == SEL_EEPROM && !self.eeprom.is_empty() {
                let address = usize::from(u16::from_le_bytes([buffer[1], buffer[2]]));
                for (index, byte) in buffer[1..].iter_mut().enumerate() {
                    *byte = self.eeprom.get(address + index).copied().unwrap_or(0xFF);
                }
            }
            Ok(())
        }
        fn payload_len(&self) -> usize {
            35
        }
    }

    #[derive(Debug, Default)]
    struct FakeCamera {
        grabs: usize,
        stops: usize,
        fail_start: bool,
        fail_stop: bool,
    }

    impl FrameSource for FakeCamera {
        fn start(&mut self) -> Result<()> {
            if self.fail_start {
                Err(Error::Rejected("test stream startup failure".into()))
            } else {
                Ok(())
            }
        }
        fn stop(&mut self) -> Result<()> {
            self.stops += 1;
            if self.fail_stop {
                Err(Error::Rejected("test stream shutdown failure".into()))
            } else {
                Ok(())
            }
        }
        fn next_frame(&mut self, _t: Duration) -> Result<Frame> {
            self.grabs += 1;
            Ok(Frame {
                width: 2592,
                height: 1944,
                format: PixelFormat::Jpeg,
                source: LightSource::White,
                data: vec![0xFF, 0xD8],
            })
        }
    }

    fn scanner() -> Scanner<Fake, FakeCamera> {
        Scanner::new(Fake::default(), FakeCamera::default()).with_settle(Duration::from_millis(0))
    }

    fn profiled_scanner() -> Scanner<Fake, FakeCamera> {
        let mut s = scanner();
        s.transport.eeprom = vec![0xFF; 0x4300];
        let mut tables = Vec::new();
        // Real layout: common then A1 General, A2 IR, A3 ID. Distinct gains
        // and lens coefficients ensure selecting a similarly-sized ID table
        // cannot pass this regression check.
        for (index, count) in [52, 29, 29, 29].into_iter().enumerate() {
            for entry in 0..count {
                let register: u16 = if entry == count - 1 { 0x350B } else { 0x5800 + entry as u16 };
                let value = [7u16, 0x14, 0x40, 0x15][index];
                tables.extend_from_slice(&register.to_be_bytes());
                tables.extend_from_slice(&value.to_be_bytes());
            }
            if index < 3 {
                tables.extend_from_slice(&[0xA1 + index as u8, 0, 0, 0, 0x74]);
            }
        }
        tables.extend_from_slice(&[0xFF; 4]);
        let start = usize::from(eeprom::TABLES_BASE) + 0x0D;
        s.transport.eeprom[start..start + tables.len()].copy_from_slice(&tables);
        for (index, address) in eeprom::GENERAL_AWB.into_iter().enumerate() {
            let at = usize::from(address);
            s.transport.eeprom[at..at + 2].copy_from_slice(&(0x3400 + index as u16).to_be_bytes());
            s.transport.eeprom[at + 2..at + 4]
                .copy_from_slice(&(0x10 + index as u16).to_be_bytes());
        }
        for (index, address) in eeprom::ID_AWB.into_iter().enumerate() {
            let at = usize::from(address);
            s.transport.eeprom[at..at + 2].copy_from_slice(&(0x3400 + index as u16).to_be_bytes());
            s.transport.eeprom[at + 2..at + 4]
                .copy_from_slice(&(0x20 + index as u16).to_be_bytes());
        }
        for (address, word) in [
            (eeprom::WHITE_EXPOSURE, [0x35, 0x01, 0, 0x45]),
            (eeprom::ID_EXPOSURE, [0x35, 0x01, 0, 0x2B]),
            (eeprom::IR_GAIN, [0x35, 0x0B, 0, 0x40]),
        ] {
            let at = usize::from(address);
            s.transport.eeprom[at..at + 4].copy_from_slice(&word);
        }
        s
    }

    #[test]
    fn general_shading_profile_uses_matching_factory_table_awb_and_exposure() {
        let mut s = profiled_scanner();
        s.calibration = s.read_calibration().unwrap();
        assert_eq!(s.calibration.white[28].value, 0x14, "General A1, not ID A3");
        assert_eq!(s.calibration.infrared[28].value, 0x40);
        assert_eq!(s.calibration.white_exposure.unwrap().value, 0x45, "General exposure, not ID");
        assert_eq!(s.calibration.white_balance.len(), 6);

        for source in [LightSource::White, LightSource::Infrared] {
            s.transport.writes.clear();
            s.capture(source).unwrap();
            let lamp_values: Vec<_> = s
                .transport
                .writes
                .iter()
                .filter(|(selector, payload)| {
                    *selector == SEL_CONTROL.get()
                        && [key::WHITE_LAMP, key::IR_LAMP].contains(&payload[0])
                })
                .map(|(_, payload)| (payload[0], payload[1]))
                .collect();
            assert_eq!(
                lamp_values,
                vec![
                    (key::WHITE_LAMP, u8::from(source == LightSource::White)),
                    (key::IR_LAMP, u8::from(source == LightSource::Infrared))
                ]
            );
            let writes: Vec<_> = s
                .transport
                .writes
                .iter()
                .filter(|(selector, _)| *selector == SEL_SENSOR_REG.get())
                .map(|(_, payload)| {
                    crate::eeprom::RegisterWrite::from_bytes(payload[..4].try_into().unwrap())
                })
                .collect();
            assert_eq!(&writes[..6], &s.calibration.white_balance);
            let table = if source == LightSource::White {
                &s.calibration.white
            } else {
                &s.calibration.infrared
            };
            assert_eq!(&writes[6..35], table);
            let trim = if source == LightSource::White {
                s.calibration.white_exposure
            } else {
                s.calibration.infrared_gain
            };
            assert_eq!(writes.last().copied(), trim, "trim follows all AWB and table writes");
        }
    }

    #[test]
    fn malformed_awb_cannot_program_an_unrelated_sensor_register() {
        let mut s = profiled_scanner();
        let at = usize::from(eeprom::GENERAL_AWB[0]);
        s.transport.eeprom[at..at + 2].copy_from_slice(&0x3008u16.to_be_bytes());
        assert!(matches!(s.read_calibration(), Err(Error::Malformed(_))));
    }

    #[test]
    fn identity_selects_all_white_factory_settings_but_preserves_infrared() {
        let mut s = profiled_scanner();
        assert_eq!(s.capture_profile(), CaptureProfile::General);
        s.set_capture_profile(CaptureProfile::Identity).unwrap();
        assert_eq!(s.capture_profile().shading_file(), "Calibration_General_ID.shd");
        assert_eq!(s.calibration.white[28].value, 0x15, "ID A3");
        assert_eq!(s.calibration.white_exposure.unwrap().value, 0x2B);
        assert_eq!(s.calibration.white_balance[0].value, 0x20);
        assert_eq!(s.calibration.infrared_white_balance[0].value, 0x10);
        for source in [LightSource::White, LightSource::Infrared] {
            s.transport.writes.clear();
            s.capture(source).unwrap();
            let lamp_values: Vec<_> = s
                .transport
                .writes
                .iter()
                .filter(|(selector, payload)| {
                    *selector == SEL_CONTROL.get()
                        && [key::WHITE_LAMP, key::IR_LAMP].contains(&payload[0])
                })
                .map(|(_, payload)| (payload[0], payload[1]))
                .collect();
            assert_eq!(
                lamp_values,
                vec![
                    (key::WHITE_LAMP, 0),
                    (key::IR_LAMP, u8::from(source == LightSource::Infrared))
                ]
            );
            let writes: Vec<_> = s
                .transport
                .writes
                .iter()
                .filter(|(selector, _)| *selector == SEL_SENSOR_REG.get())
                .map(|(_, payload)| {
                    crate::eeprom::RegisterWrite::from_bytes(payload[..4].try_into().unwrap())
                })
                .collect();
            let (awb, table, trim) = if source == LightSource::White {
                (&s.calibration.white_balance, &s.calibration.white, s.calibration.white_exposure)
            } else {
                (
                    &s.calibration.infrared_white_balance,
                    &s.calibration.infrared,
                    s.calibration.infrared_gain,
                )
            };
            assert_eq!(&writes[..6], awb);
            assert_eq!(&writes[6..35], table);
            assert_eq!(writes.last().copied(), trim);
        }
        s.set_capture_profile(CaptureProfile::General).unwrap();
        assert_eq!(s.capture_profile().shading_file(), "Calibration_General.shd");
        assert_eq!(s.calibration.white[28].value, 0x14);
        assert_eq!(s.calibration.white_exposure.unwrap().value, 0x45);
        assert_eq!(s.calibration.white_balance[0].value, 0x10);
    }

    #[test]
    fn malformed_identity_profile_leaves_general_profile_unchanged() {
        let mut s = profiled_scanner();
        s.set_capture_profile(CaptureProfile::General).unwrap();
        let at = usize::from(eeprom::ID_AWB[0]);
        s.transport.eeprom[at..at + 2].copy_from_slice(&0x3008u16.to_be_bytes());
        assert!(s.set_capture_profile(CaptureProfile::Identity).is_err());
        assert_eq!(s.capture_profile(), CaptureProfile::General);
        assert_eq!(s.calibration.white_exposure.unwrap().value, 0x45);
    }

    #[test]
    fn exposure_compensation_rounds_factory_trim_and_never_compounds() {
        let mut s = profiled_scanner();
        s.set_capture_profile_with_exposure_percent(CaptureProfile::Identity, 120).unwrap();
        assert_eq!(s.calibration.white_exposure.unwrap().value, 52);
        let selected = s.calibration.clone();
        s.set_capture_profile_with_exposure_percent(CaptureProfile::Identity, 120).unwrap();
        assert_eq!(s.calibration, selected);
        s.set_white_exposure(200);
        s.set_capture_profile_with_exposure_percent(CaptureProfile::Identity, 120).unwrap();
        assert_eq!(s.calibration, selected);
        s.set_capture_profile(CaptureProfile::Identity).unwrap();
        assert_eq!(s.calibration.white_exposure.unwrap().value, 43);
        s.set_capture_profile(CaptureProfile::General).unwrap();
        assert_eq!(s.calibration.white_exposure.unwrap().value, 69);
    }

    #[test]
    fn rejected_exposure_selection_preserves_the_entire_previous_configuration() {
        let mut s = profiled_scanner();
        s.set_capture_profile_with_exposure_percent(CaptureProfile::Identity, 120).unwrap();
        let selected = s.calibration.clone();
        for percent in [0, 1000, u16::MAX] {
            assert!(
                s.set_capture_profile_with_exposure_percent(CaptureProfile::General, percent)
                    .is_err()
            );
            assert_eq!(s.capture_profile(), CaptureProfile::Identity);
            assert_eq!(s.calibration, selected);
        }
        // A positive percentage that rounds this factory trim to zero is invalid.
        assert!(s.set_capture_profile_with_exposure_percent(CaptureProfile::Identity, 1).is_err());
        assert_eq!(s.calibration, selected);

        let at = usize::from(eeprom::WHITE_EXPOSURE);
        for word in
            [[0xFF, 0xFF, 0xFF, 0xFF], [0x35, 0x0B, 0, 69], [0x35, 1, 1, 0], [0x35, 1, 0, 0]]
        {
            s.transport.eeprom[at..at + 4].copy_from_slice(&word);
            assert!(
                s.set_capture_profile_with_exposure_percent(CaptureProfile::General, 120).is_err()
            );
            assert_eq!(s.capture_profile(), CaptureProfile::Identity);
            assert_eq!(s.calibration, selected);
        }
    }

    #[test]
    fn diagnostic_white_lamp_override_does_not_change_infrared_or_factory_trim() {
        let mut s = profiled_scanner();
        s.set_capture_profile(CaptureProfile::Identity).unwrap();
        s.set_diagnostic_white_lamp(Some(true));
        for (source, expected) in [(LightSource::White, 1), (LightSource::Infrared, 0)] {
            s.transport.writes.clear();
            s.capture(source).unwrap();
            let (_, payload) = s
                .transport
                .writes
                .iter()
                .find(|(selector, payload)| {
                    *selector == SEL_CONTROL.get() && payload[0] == key::WHITE_LAMP
                })
                .unwrap();
            assert_eq!(payload[1], expected);
        }
        assert_eq!(s.calibration.white_exposure.unwrap().value, 0x2B);
        s.set_diagnostic_white_lamp(None);
        s.transport.writes.clear();
        s.capture(LightSource::White).unwrap();
        assert!(s.transport.writes.iter().any(|(selector, payload)| {
            *selector == SEL_CONTROL.get() && payload[..2] == [key::WHITE_LAMP, 0]
        }));
    }

    #[test]
    fn identity_lamp_preserves_other_bridge_bits_and_clears_for_ir_general_and_cleanup() {
        let mut s = profiled_scanner();
        s.transport.bridge_register_7 = 0x25;
        s.set_capture_profile(CaptureProfile::Identity).unwrap();
        s.capture(LightSource::White).unwrap();
        assert_eq!(s.transport.bridge_register_7, 0x65);
        s.capture(LightSource::Infrared).unwrap();
        assert_eq!(s.transport.bridge_register_7, 0x25);
        s.capture(LightSource::White).unwrap();
        s.set_capture_profile(CaptureProfile::General).unwrap();
        s.capture(LightSource::White).unwrap();
        assert_eq!(s.transport.bridge_register_7, 0x25);
        s.set_capture_profile(CaptureProfile::Identity).unwrap();
        s.scan(&[LightSource::White]).unwrap();
        assert_eq!(s.transport.bridge_register_7, 0x25);
        assert!(!s.identity_lamp_active);
    }

    #[test]
    fn general_clears_external_identity_light_only_with_vendor_idlight_flag() {
        for (id_light, expected) in [(false, 0x65), (true, 0x25)] {
            let mut s = profiled_scanner();
            s.apply_settings(&crate::settings::Settings {
                id_light,
                ..crate::settings::Settings::default()
            });
            s.set_settle(Duration::ZERO);
            s.transport.bridge_register_7 = 0x65;
            s.capture(LightSource::White).unwrap();
            assert_eq!(s.transport.bridge_register_7, expected);
        }
    }

    #[test]
    fn sensor_settings_clear_generic_camera_overrides() {
        let mut s = scanner();
        s.set_white_controls(vec![("Saturation".into(), 13)]);
        s.apply_settings(&crate::settings::Settings::default());
        assert!(s.white_controls.is_empty());
    }

    #[test]
    fn generic_settings_apply_only_supported_manual_camera_values() {
        #[derive(Debug)]
        struct Controls;
        impl FrameSource for Controls {
            fn start(&mut self) -> Result<()> {
                Ok(())
            }
            fn stop(&mut self) -> Result<()> {
                Ok(())
            }
            fn next_frame(&mut self, _: Duration) -> Result<Frame> {
                unreachable!()
            }
            fn controls(&self) -> Vec<passport_scanner_core::Control> {
                [("Gamma", 1, 12), ("Brightness", 0, 80), ("Saturation", 0, 20)]
                    .into_iter()
                    .map(|(name, min, max)| passport_scanner_core::Control {
                        name: name.into(),
                        min,
                        max,
                        value: min,
                    })
                    .collect()
            }
        }
        let mut s = Scanner::new(Fake::default(), Controls);
        let settings = crate::settings::Settings {
            sensor_controls: false,
            white: crate::settings::CameraProps::parse("0,1,0,0,0,0,0,30,2,128,13,0,0,0"),
            ..crate::settings::Settings::default()
        };
        s.apply_settings(&settings);
        // Gamma requests automatic mode, brightness is out of range, and
        // only saturation is an explicit manual value this backend supports.
        assert_eq!(s.white_controls, vec![("Saturation".into(), 13)]);
    }

    #[test]
    fn presence_reads_both_sensors() {
        let mut s = scanner();
        s.transport.sensor_a = 1;
        s.transport.sensor_b = 0;
        assert_eq!(s.document_present().unwrap(), DocumentPresence::Partial);

        s.transport.sensor_b = 1;
        assert_eq!(s.document_present().unwrap(), DocumentPresence::Full);
    }

    #[test]
    fn a_scan_takes_one_frame_per_source() {
        let mut s = scanner();
        let frames = s.scan(&[LightSource::White, LightSource::Infrared]).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].source, LightSource::White);
        assert_eq!(frames[1].source, LightSource::Infrared);
    }

    #[test]
    fn single_capture_uses_the_same_initialisation_and_cleanup_as_a_scan() {
        let mut s = scanner();
        s.calibration.common.push(crate::eeprom::RegisterWrite { register: 0x3503, value: 7 });
        s.capture_once(LightSource::White).unwrap();
        assert_eq!(s.capture.stops, 1);
        assert!(s.transport.writes.iter().any(|(selector, payload)| {
            *selector == SEL_SENSOR_REG.get() && payload[..4] == [0x35, 3, 0, 7]
        }));
    }

    #[test]
    fn lamp_cleanup_attempts_both_controls_after_a_bridge_failure() {
        let mut s = scanner();
        s.identity_lamp_active = true;
        s.transport.bridge_register_7 = 0x40;
        s.transport.fail_bridge_write = true;
        s.transport.fail_control_off = Some(key::WHITE_LAMP);
        let error = s.lights_off().unwrap_err().to_string();
        assert!(error.contains("test bridge write failed"));
        assert!(s.identity_lamp_active, "failed shutdown must remain eligible for retry");
        for lamp in [key::WHITE_LAMP, key::IR_LAMP] {
            assert!(s.transport.writes.iter().any(|(selector, payload)| {
                *selector == SEL_CONTROL.get() && payload[..2] == [lamp, key::OFF]
            }));
        }
    }

    #[test]
    fn successful_capture_reports_lamp_shutdown_failure() {
        let mut s = scanner();
        s.transport.fail_control_off = Some(key::WHITE_LAMP);
        let error = s.scan(&[LightSource::White]).unwrap_err().to_string();
        assert!(error.contains("test control F1 off failed"));
        assert!(s.capture.grabs > 0, "capture itself succeeded");
        assert_eq!(s.capture.stops, 1);
        assert_eq!(&s.transport.writes.last().unwrap().1[..2], &[key::IR_LAMP, key::OFF]);
    }

    #[test]
    fn stream_shutdown_error_precedes_lamp_error_without_skipping_cleanup() {
        let mut s = scanner();
        s.capture.fail_stop = true;
        s.transport.fail_control_off = Some(key::WHITE_LAMP);
        let error = s.scan(&[LightSource::White]).unwrap_err().to_string();
        assert!(error.contains("test stream shutdown failure"));
        for command in [key::IMAGING_ENABLE, key::WHITE_LAMP, key::IR_LAMP] {
            assert!(s.transport.writes.iter().any(|(selector, payload)| {
                *selector == SEL_CONTROL.get() && payload[..2] == [command, key::OFF]
            }));
        }
    }

    #[test]
    fn startup_failure_still_disables_imaging_and_lamps() {
        let mut s = scanner();
        s.capture.fail_start = true;
        s.capture.fail_stop = true;
        s.transport.fail_control_off = Some(key::WHITE_LAMP);
        let error = s.scan(&[LightSource::White]).unwrap_err().to_string();
        assert!(error.contains("test stream startup failure"));
        assert_eq!(s.capture.stops, 1);
        for key in [key::IMAGING_ENABLE, key::WHITE_LAMP, key::IR_LAMP] {
            let last = s
                .transport
                .writes
                .iter()
                .rev()
                .find(|(selector, payload)| *selector == SEL_CONTROL.get() && payload[0] == key)
                .unwrap();
            assert_eq!(last.1[1], 0);
        }
    }

    #[test]
    fn a_scan_switches_infrared_on_and_leaves_nothing_lit() {
        let mut s = scanner();
        s.scan(&[LightSource::White, LightSource::Infrared]).unwrap();

        let control: Vec<(u8, u8)> = s
            .transport
            .writes
            .iter()
            .filter(|(sel, _)| *sel == SEL_CONTROL.get())
            .map(|(_, p)| (p[0], p[1]))
            .collect();

        assert!(control.contains(&(key::IR_LAMP, key::ON)), "infrared is switched on");
        assert!(control.contains(&(key::WHITE_LAMP, key::ON)), "white is switched on");
        assert!(control.contains(&(key::IMAGING_ENABLE, ENABLE)), "imaging path is freed");

        // Nothing stays lit: the last write to each lamp key turns it off.
        for k in [key::SENSOR_A, key::IR_LAMP] {
            let last = control.iter().rfind(|(ck, _)| *ck == k);
            assert_eq!(last.map(|(_, v)| *v), Some(key::OFF), "key {k:#04X} left on");
        }
    }

    #[test]
    fn ultraviolet_is_refused_rather_than_silently_skipped() {
        let mut s = scanner();
        assert!(matches!(s.capture(LightSource::Ultraviolet), Err(Error::Unsupported(_))));
    }

    #[test]
    fn eeprom_reads_span_several_exchanges() {
        let mut s = scanner();
        // 35-byte payload leaves 34 usable bytes per reply, so 80 bytes needs three.
        let data = s.read_eeprom(0x4000, 80).unwrap();
        assert_eq!(data.len(), 80);
    }
}
