//! The shipped driver's own per-model settings.
//!
//! Timings, lamp behaviour and camera properties are not constants of the
//! hardware. They are configuration, and the vendor ships them in
//! `Camera.ini` beside its application — one section per model, selected by
//! USB vendor and product id. Guessing at them from behaviour, which is what
//! this code did for a while, produces numbers that are nearly right and
//! occasionally wrong; reading them produces the numbers the device was built
//! around.
//!
//! So that file is read where it is installed, and what it says wins over the
//! defaults here. Those defaults are the X-Mini 2000's own values, kept so
//! that a machine without the vendor's software still behaves sensibly.
//!
//! The file is UTF-16, which `GetPrivateProfileString` reads transparently on
//! Windows and nothing else does, so the decoding is done here.

use std::{collections::HashMap, path::Path, time::Duration};

/// Where the vendor installs its application, and so its configuration.
pub const VENDOR_DIR: &str = r"C:\Program Files\Plustek\Plustek SecureScan Series V6.4.7.1";

/// The file holding the per-model settings.
pub const CONFIG_FILE: &str = "Camera.ini";

/// How long to let the image settle after changing illumination.
///
/// `DelayTime-5M-<source>` — 400 ms for white and infrared on this model,
/// 2000 ms for ultraviolet. Measured against the running driver twice, at
/// 400 ms and 401 ms, which is how this was pinned before the file was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delays {
    /// After switching to the visible lamp.
    pub white: Duration,
    /// After switching to infrared.
    pub infrared: Duration,
    /// After switching to ultraviolet.
    pub ultraviolet: Duration,
    /// After a document is first detected, before reading the sensors again.
    ///
    /// `FirstDelay` — 500 ms. A card is still moving when it first crosses a
    /// sensor, and this is how long the shipped driver waits before deciding
    /// anything about it.
    pub first: Duration,
    /// How long the buzzer sounds. `BuzzerDelay` — 300 ms.
    pub buzzer: Duration,
    /// How long to wait for a frame before giving up. `TimeOut` — 6000 ms.
    pub timeout: Duration,
}

impl Default for Delays {
    fn default() -> Self {
        Self {
            white: Duration::from_millis(400),
            infrared: Duration::from_millis(400),
            ultraviolet: Duration::from_millis(2000),
            first: Duration::from_millis(500),
            buzzer: Duration::from_millis(300),
            timeout: Duration::from_millis(6000),
        }
    }
}

/// Requested mode of a vendor camera property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraMode {
    /// Apply a numeric value with automatic adjustment disabled.
    Manual(i64),
    /// Let the camera adjust the property automatically.
    Auto,
}

/// The capture stack's image controls, as the shipped driver sets them.
///
/// `CameraProp-5M-<source>`: seven auto-mode flags, then seven values, in the
/// order the file's own comment gives — iris, gamma, brightness, saturation,
/// sharpness, hue, backlight compensation.
///
/// Not every one of these is a control this device actually carries, and the
/// values are not all in the ranges it reports: the file asks for brightness
/// 128 where the device offers 0 to 80, and gamma 64 where it offers 1 to 12.
/// These properties belong to the generic camera path; `CmdEGW` selects a
/// separate sensor-command path on X-Mini 2000. On the generic path, a manual
/// value outside the backend's reported range is left alone rather than
/// clamped to an unrelated value. Automatic requests are retained explicitly
/// but cannot yet be applied through the capture interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CameraProps {
    /// Iris, which this device does not carry as a camera-control property.
    pub iris: Option<CameraMode>,
    /// Gamma.
    pub gamma: Option<CameraMode>,
    /// Brightness.
    pub brightness: Option<CameraMode>,
    /// Saturation.
    pub saturation: Option<CameraMode>,
    /// Sharpness.
    pub sharpness: Option<CameraMode>,
    /// Hue.
    pub hue: Option<CameraMode>,
    /// Backlight compensation.
    pub backlight: Option<CameraMode>,
}

impl CameraProps {
    /// Manual values, paired with the control names a capture stack uses.
    ///
    /// Iris is left out: it is not a video-processing property, and this
    /// device reports no camera-control one either.
    #[must_use]
    pub fn controls(&self) -> Vec<(&'static str, i64)> {
        [
            ("Gamma", self.gamma),
            ("Brightness", self.brightness),
            ("Saturation", self.saturation),
            ("Sharpness", self.sharpness),
            ("Hue", self.hue),
            ("BacklightCompensation", self.backlight),
        ]
        .into_iter()
        .filter_map(|(name, value)| match value {
            Some(CameraMode::Manual(value)) => Some((name, value)),
            _ => None,
        })
        .collect()
    }

    /// Automatic video-property requests unsupported by the current capture
    /// interface. They must never be silently turned into manual values.
    #[must_use]
    pub fn automatic_controls(&self) -> Vec<&'static str> {
        [
            ("Iris", self.iris),
            ("Gamma", self.gamma),
            ("Brightness", self.brightness),
            ("Saturation", self.saturation),
            ("Sharpness", self.sharpness),
            ("Hue", self.hue),
            ("BacklightCompensation", self.backlight),
        ]
        .into_iter()
        .filter_map(|(name, value)| (value == Some(CameraMode::Auto)).then_some(name))
        .collect()
    }

    /// Read one `CameraProp` line: seven flags, then seven values.
    ///
    /// Zero selects manual mode using the numeric value; one selects auto.
    /// The current FrameSource interface cannot request auto mode, so
    /// [`Self::controls`] returns only explicit manual values.
    #[must_use]
    pub fn parse(line: &str) -> Self {
        let Ok(fields) =
            line.split(',').map(|f| f.trim().parse::<i64>()).collect::<Result<Vec<_>, _>>()
        else {
            return Self::default();
        };
        if fields.len() < 14 {
            return Self::default();
        }
        let pick = |i: usize| match fields[i] {
            0 => Some(CameraMode::Manual(fields[i + 7])),
            1 => Some(CameraMode::Auto),
            _ => None,
        };
        Self {
            iris: pick(0),
            gamma: pick(1),
            brightness: pick(2),
            saturation: pick(3),
            sharpness: pick(4),
            hue: pick(5),
            backlight: pick(6),
        }
    }
}

/// Everything read for one model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The model's section name in the file, when it came from one.
    pub model: Option<String>,
    /// First `CmdEGW` field: use sensor commands instead of generic camera
    /// properties. X-Mini 2000 ships `CmdEGW=1,0`.
    pub sensor_controls: bool,
    /// `IDLight=1`: General illumination must clear inherited ID bridge light.
    pub id_light: bool,
    /// Settling and waiting times.
    pub delays: Delays,
    /// Image controls for a visible-light capture.
    pub white: CameraProps,
    /// Image controls for an infrared capture.
    pub infrared: CameraProps,
    /// Whether to sound the buzzer when a scan completes. `Buzzer`.
    pub buzzer: bool,
    /// Source selector used by vendor ID calibration (`IDCalibrationSource`).
    /// This does not select General versus General_ID shading.
    pub identity_calibration: u8,
}

impl Default for Settings {
    fn default() -> Self {
        // The X-Mini 2000's own values, for a machine with no vendor install.
        Self {
            model: None,
            sensor_controls: true,
            // Do not add bridge writes without the model's explicit flag.
            id_light: false,
            delays: Delays::default(),
            white: CameraProps::parse("1,1,1,1,1,1,1,30,64,128,13,0,0,0"),
            infrared: CameraProps::parse("1,1,1,1,1,1,1,120,64,128,13,0,0,0"),
            buzzer: true,
            identity_calibration: 0,
        }
    }
}

impl Settings {
    /// Read the settings for one unit from the vendor's installed
    /// configuration, falling back to the defaults for anything absent.
    ///
    /// Never fails: a machine without the vendor's software gets the defaults,
    /// which is a working scanner rather than an error.
    #[must_use]
    pub fn load(vendor: u16, product: u16) -> Self {
        Self::load_from(Path::new(VENDOR_DIR).join(CONFIG_FILE), vendor, product)
    }

    /// The same, from a named file.
    #[must_use]
    pub fn load_from(path: impl AsRef<Path>, vendor: u16, product: u16) -> Self {
        let Ok(bytes) = std::fs::read(path.as_ref()) else {
            return Self::default();
        };
        Self::parse(&decode(&bytes), vendor, product)
    }

    /// Read the settings for one unit out of the file's text.
    #[must_use]
    pub fn parse(text: &str, vendor: u16, product: u16) -> Self {
        let sections = sections(text);
        let wanted = format!("VID_{vendor:04X}&PID_{product:04X}");

        // [ID] maps an index to a USB id, [Name] maps the same index to the
        // section holding that model's settings.
        let index = sections
            .get("ID")
            .and_then(|ids| {
                ids.iter().find(|(_, v)| v.eq_ignore_ascii_case(&wanted)).map(|(k, _)| k.clone())
            })
            .unwrap_or_default();
        let Some(model) = sections.get("Name").and_then(|names| names.get(&index)).cloned() else {
            return Self::default();
        };
        let Some(values) = sections.get(&model) else {
            return Self::default();
        };

        let mut settings = Self {
            model: Some(model.clone()),
            sensor_controls: values
                .get("CmdEGW")
                .and_then(|v| v.split(',').next())
                .and_then(|v| v.trim().parse::<u8>().ok())
                .is_some_and(|v| v != 0),
            id_light: values.get("IDLight").is_some_and(|v| v.trim() == "1"),
            ..Self::default()
        };
        let ms = |key: &str| {
            values.get(key).and_then(|v| v.parse::<u64>().ok()).map(Duration::from_millis)
        };
        if let Some(d) = ms("DelayTime-5M-White") {
            settings.delays.white = d;
        }
        if let Some(d) = ms("DelayTime-5M-IR") {
            settings.delays.infrared = d;
        }
        if let Some(d) = ms("DelayTime-5M-UV") {
            settings.delays.ultraviolet = d;
        }
        if let Some(d) = ms("FirstDelay") {
            settings.delays.first = d;
        }
        if let Some(d) = ms("BuzzerDelay") {
            settings.delays.buzzer = d;
        }
        if let Some(d) = ms("TimeOut") {
            settings.delays.timeout = d;
        }
        if let Some(line) = values.get("CameraProp-5M-White") {
            settings.white = CameraProps::parse(line);
        }
        if let Some(line) = values.get("CameraProp-5M-IR") {
            settings.infrared = CameraProps::parse(line);
        }
        if let Some(v) = values.get("Buzzer").and_then(|v| v.parse::<u8>().ok()) {
            settings.buzzer = v != 0;
        }
        if let Some(v) = values.get("IDCalibrationSource").and_then(|v| v.parse::<u8>().ok()) {
            settings.identity_calibration = v;
        }
        settings
    }
}

/// Decode a configuration file, whatever the vendor wrote it as.
///
/// These are UTF-16 with a byte-order mark, which only the Windows profile
/// API reads transparently. Anything else is treated as Latin-1, which cannot
/// fail and leaves ASCII keys intact.
fn decode(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        return String::from_utf16_lossy(
            &bytes[2..]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect::<Vec<u16>>(),
        );
    }
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        return String::from_utf16_lossy(
            &bytes[2..]
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect::<Vec<u16>>(),
        );
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|b| *b as char).collect(),
    }
}

/// Split an INI into sections of key-value pairs.
fn sections(text: &str) -> HashMap<String, HashMap<String, String>> {
    let mut out: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = name.trim().to_owned();
            out.entry(current.clone()).or_default();
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            out.entry(current.clone())
                .or_default()
                .insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of the vendor's file, with the reference unit's own values.
    const SHIPPED: &str = "\
[ID]\r
0=VID_07B3&PID_1505\r
6=VID_07B3&PID_1A05\r
\r
[Name]\r
0=X50\r
6=X-Mini 2000\r
\r
[X50]\r
Buzzer=0\r
DelayTime-5M-White=1234\r
\r
[X-Mini 2000]\r
Buzzer=1\r
BuzzerDelay=300\r
FirstDelay=500\r
TimeOut=6000\r
IDCalibrationSource=0\r
DelayTime-5M-White=400\r
DelayTime-5M-IR=400\r
DelayTime-5M-UV=2000\r
;IRISAuto,GammaAuto,BrightnessAuto,SaturationAuto,SharpnessAuto,HueAuto,BacklightCompensationAuto\r
CameraProp-5M-White=1,1,1,1,1,1,1,30,64,128,13,0,0,0\r
CameraProp-5M-IR=1,1,1,1,1,1,1,120,64,128,13,0,0,0\r
";

    #[test]
    fn the_unit_is_found_through_the_two_index_tables() {
        let s = Settings::parse(SHIPPED, 0x07B3, 0x1A05);
        assert_eq!(s.model.as_deref(), Some("X-Mini 2000"));
        assert_eq!(s.delays.white, Duration::from_millis(400));
    }

    #[test]
    fn a_different_unit_gets_its_own_section_not_this_one() {
        let s = Settings::parse(SHIPPED, 0x07B3, 0x1505);
        assert_eq!(s.model.as_deref(), Some("X50"));
        assert_eq!(s.delays.white, Duration::from_millis(1234));
        assert!(!s.buzzer);
    }

    #[test]
    fn an_unknown_unit_falls_back_to_the_defaults() {
        let s = Settings::parse(SHIPPED, 0xFFFF, 0xFFFF);
        assert_eq!(s.model, None);
        assert_eq!(s, Settings { model: None, ..Settings::default() });
    }

    #[test]
    fn the_delays_are_the_ones_the_file_gives() {
        let s = Settings::parse(SHIPPED, 0x07B3, 0x1A05);
        assert_eq!(s.delays.infrared, Duration::from_millis(400));
        assert_eq!(s.delays.ultraviolet, Duration::from_millis(2000));
        assert_eq!(s.delays.first, Duration::from_millis(500));
        assert_eq!(s.delays.buzzer, Duration::from_millis(300));
        assert_eq!(s.delays.timeout, Duration::from_millis(6000));
    }

    #[test]
    fn the_camera_properties_are_read_as_flags_then_values() {
        let p = CameraProps::parse("1,1,1,1,1,1,1,30,64,128,13,0,0,0");
        assert_eq!(p.iris, Some(CameraMode::Auto));
        assert_eq!(p.gamma, Some(CameraMode::Auto));
        assert_eq!(p.brightness, Some(CameraMode::Auto));
        assert_eq!(p.saturation, Some(CameraMode::Auto));
        assert_eq!(p.sharpness, Some(CameraMode::Auto));
        assert!(p.controls().is_empty());
        assert_eq!(p.automatic_controls().len(), 7);
    }

    #[test]
    fn a_cleared_flag_selects_manual_mode_and_its_value() {
        let p = CameraProps::parse("0,1,0,1,0,0,0,30,64,128,13,0,0,0");
        assert_eq!(p.iris, Some(CameraMode::Manual(30)));
        assert_eq!(p.gamma, Some(CameraMode::Auto));
        assert_eq!(p.brightness, Some(CameraMode::Manual(128)));
        assert_eq!(p.saturation, Some(CameraMode::Auto));
        assert_eq!(
            p.controls(),
            vec![("Brightness", 128), ("Sharpness", 0), ("Hue", 0), ("BacklightCompensation", 0)]
        );
    }

    #[test]
    fn a_short_or_broken_line_sets_nothing() {
        assert_eq!(CameraProps::parse("1,1,1"), CameraProps::default());
        assert_eq!(CameraProps::parse(""), CameraProps::default());
        assert_eq!(
            CameraProps::parse("0,0,bad,0,0,0,0,30,64,128,13,0,0,0,99"),
            CameraProps::default()
        );
    }

    #[test]
    fn command_egw_selects_sensor_controls_from_its_first_field() {
        let sensor = SHIPPED.replace("Buzzer=1", "CmdEGW=1,0\nBuzzer=1");
        assert!(Settings::parse(&sensor, 0x07B3, 0x1A05).sensor_controls);
        let generic = sensor.replace("CmdEGW=1,0", "CmdEGW=0,0");
        assert!(!Settings::parse(&generic, 0x07B3, 0x1A05).sensor_controls);
        assert!(!Settings::parse(SHIPPED, 0x07B3, 0x1A05).sensor_controls);
        assert!(Settings::default().sensor_controls);
    }

    #[test]
    fn idlight_bridge_cleanup_is_explicit_and_model_scoped() {
        let text = SHIPPED.replace("Buzzer=1", "IDLight=1\nBuzzer=1");
        assert!(Settings::parse(&text, 0x07B3, 0x1A05).id_light);
        assert!(!Settings::parse(&text, 0x07B3, 0x1505).id_light);
        assert!(!Settings::parse(SHIPPED, 0x07B3, 0x1A05).id_light);
        assert!(!Settings::default().id_light);
    }

    #[test]
    fn iris_is_left_out_of_the_controls_because_the_device_has_none() {
        let p = CameraProps::parse("0,0,0,0,0,0,0,30,64,128,13,0,0,0");
        let names: Vec<&str> = p.controls().iter().map(|(n, _)| *n).collect();
        assert!(!names.contains(&"Iris"));
        assert!(names.contains(&"Saturation"));
        assert_eq!(
            p.controls().iter().find(|(n, _)| *n == "Saturation").map(|(_, v)| *v),
            Some(13)
        );
    }

    #[test]
    fn the_identity_calibration_source_is_read_rather_than_assumed() {
        // This is a calibration source selector, not a shading-profile choice.
        assert_eq!(Settings::parse(SHIPPED, 0x07B3, 0x1A05).identity_calibration, 0);
    }

    #[test]
    fn a_utf16_file_is_decoded_the_way_the_vendor_writes_one() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "[A]\r\nk=v\r\n".encode_utf16() {
            bytes.extend(unit.to_le_bytes());
        }
        assert_eq!(decode(&bytes), "[A]\r\nk=v\r\n");
    }

    #[test]
    fn comments_and_blank_lines_are_not_settings() {
        let parsed = sections("[A]\n; a comment = not a key\n\nk = v\n");
        assert_eq!(parsed["A"].get("k").map(String::as_str), Some("v"));
        assert_eq!(parsed["A"].len(), 1);
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let s = Settings::load_from("no-such-file.ini", 0x07B3, 0x1A05);
        assert_eq!(s.model, None);
    }
}
