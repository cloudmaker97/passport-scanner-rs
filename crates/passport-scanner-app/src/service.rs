//! A single scanner owner and shared session for desktop, web and MCP clients.
#[cfg(test)]
#[path = "service_session_tests.rs"]
mod session_tests;
use crate::preferences::CaptureSelection;
use passport_scanner_core::{Error, Result};
use passport_scanner_protocol::{Command, Device, Event, Scan};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{Receiver, Sender, channel},
};

#[derive(Debug, Default)]
struct Session {
    device: Option<Device>,
    scans: Vec<Scan>,
    selected: Option<String>,
    listeners: Vec<Sender<String>>,
    settings: std::collections::BTreeMap<String, String>,
    calibration: Option<Event>,
    auto: Option<bool>,
    progress: Option<Event>,
    resumed: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Hub {
    session: Mutex<Session>,
    stop: AtomicBool,
}

impl Session {
    fn state(&self) -> Event {
        Event::State {
            device: self.device.clone(),
            scans: self.scans.clone(),
            selected: self.selected.clone(),
        }
    }
    fn emit(&mut self, event: &Event) {
        if let Ok(json) = serde_json::to_string(event) {
            self.listeners.retain(|tx| tx.send(json.clone()).is_ok());
        }
    }
}
impl Hub {
    /// Restore an explicitly supplied local restart snapshot before opening hardware.
    fn restore(&self, events: Vec<Event>) -> Result<()> {
        let mut restored = Session::default();
        let mut found_state = false;
        for event in events {
            match event {
                Event::State { scans, selected, .. } if !found_state => {
                    let mut ids = std::collections::HashSet::new();
                    if scans.iter().any(|scan| {
                        scan.id.is_empty() || scan.document.is_empty() || !ids.insert(&scan.id)
                    }) {
                        return Err(Error::Malformed(
                            "Invalid scan identifiers in restart snapshot".into(),
                        ));
                    }
                    restored.selected =
                        selected.filter(|id| scans.iter().any(|scan| &scan.id == id));
                    restored.scans = scans;
                    found_state = true;
                }
                Event::AutoScanChanged { enabled } => restored.auto = Some(enabled),
                _ => return Err(Error::Malformed("Invalid event in restart snapshot".into())),
            }
        }
        if !found_state {
            return Err(Error::Malformed("Restart snapshot has no scan state".into()));
        }
        restored.resumed = true;
        *self.session.lock().expect("session") = restored;
        Ok(())
    }
    #[cfg(test)]
    fn with_listener(listener: Sender<String>) -> Self {
        let hub = Self::default();
        hub.session.lock().expect("session").listeners.push(listener);
        hub
    }
    fn publish(&self, event: Event) {
        let mut s = self.session.lock().expect("session");
        match &event {
            Event::DeviceChanged { device } => s.device = device.clone(),
            Event::ScanCompleted { scan } => {
                s.progress = None;
                s.scans.retain(|old| old.document != scan.document || old.side != scan.side);
                s.selected = Some(scan.id.clone());
                s.scans.insert(0, *scan.clone());
            }
            Event::SettingsChanged { settings } => s.settings.extend(settings.clone()),
            Event::AutoScanChanged { enabled } => s.auto = Some(*enabled),
            Event::CalibrationChanged { .. } => {
                s.calibration = Some(event.clone());
                s.progress = None;
            }
            Event::ScanProgress { .. } => s.progress = Some(event.clone()),
            Event::Failed { .. } => s.progress = None,
            _ => {}
        }
        // Neither MRZ text nor image data belongs in diagnostic logs.
        if let Event::Failed { code, .. } = &event {
            eprintln!("scanner event: {code}");
        }
        s.emit(&event);
    }
    fn state(&self) -> Event {
        self.session.lock().expect("session").state()
    }
    fn publish_state(&self) {
        let mut s = self.session.lock().expect("session");
        let e = s.state();
        s.emit(&e);
    }
    fn select(&self, selected: Option<String>) {
        let mut s = self.session.lock().expect("session");
        s.selected = selected;
        let e = s.state();
        s.emit(&e);
    }
    fn document(&self, id: &str) -> Option<String> {
        self.session
            .lock()
            .expect("session")
            .scans
            .iter()
            .find(|scan| scan.id == id)
            .map(|scan| scan.document.clone())
    }
    fn discard(&self, id: &str) {
        let mut s = self.session.lock().expect("session");
        let document = s.scans.iter().find(|scan| scan.id == id).map(|scan| scan.document.clone());
        if let Some(document) = document {
            s.scans.retain(|scan| scan.document != document);
        }
        if s.selected.as_ref().is_some_and(|id| !s.scans.iter().any(|scan| &scan.id == id)) {
            s.selected = None;
        }
        let e = s.state();
        s.emit(&e);
    }
    fn subscribe(&self) -> Receiver<String> {
        let (tx, rx) = channel();
        let mut s = self.session.lock().expect("session");
        let mut initial = vec![
            Event::Hello { version: passport_scanner_protocol::VERSION },
            s.state(),
            Event::SettingsChanged { settings: s.settings.clone() },
        ];
        if let Some(enabled) = s.auto {
            initial.push(Event::AutoScanChanged { enabled });
        }
        if let Some(e) = &s.calibration {
            initial.push(e.clone());
        }
        if let Some(e) = &s.progress {
            initial.push(e.clone());
        }
        for e in initial {
            let _ = tx.send(serde_json::to_string(&e).expect("event JSON"));
        }
        s.listeners.push(tx);
        rx
    }
}

/// Owns the scanner thread. Front ends share this handle rather than opening cameras.
#[derive(Debug)]
pub struct Engine {
    hub: Arc<Hub>,
    asks: Sender<Ask>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}
impl Engine {
    /// Start with a validated saved or explicitly selected capture profile.
    pub fn start(profile: CaptureSelection) -> Result<Self> {
        let preference_file = crate::preferences::path()?;
        let hub = Arc::new(Hub::default());
        // Explicit maintenance restart only; ordinary launches retain their empty session.
        if let Some(path) = std::env::var_os("PASSPORT_SCANNER_RESTORE_SESSION") {
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(path)?.take(256 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
            if bytes.len() > 256 * 1024 * 1024 {
                return Err(Error::Malformed("Restart snapshot is too large".into()));
            }
            hub.restore(serde_json::from_slice(&bytes).map_err(Error::backend)?)?;
        }
        let (asks, rx) = channel();
        let shared = hub.clone();
        let worker = std::thread::Builder::new().name("passport-scanner-device".into()).spawn(
            move || {
                #[cfg(windows)]
                watch(&shared, &rx, profile, &preference_file);
                #[cfg(not(windows))]
                {
                    let _ = (rx, profile, preference_file);
                    broadcast(&shared, &failed("unsupported", "No transport for this platform."));
                }
            },
        )?;
        Ok(Self { hub, asks, worker: Mutex::new(Some(worker)) })
    }
    /// Queue a command on the device-owning thread.
    pub fn send(&self, command: Command) -> Result<()> {
        if self.hub.stop.load(Ordering::Acquire) {
            return Err(Error::Rejected("Scanner session has stopped.".into()));
        }
        let json = serde_json::to_string(&command).map_err(Error::backend)?;
        handle_command(&json, &self.hub, &self.asks);
        Ok(())
    }
    /// Receive an initial state and subsequent protocol events.
    pub fn subscribe(&self) -> Receiver<String> {
        self.hub.subscribe()
    }
    /// Current device, selected capture and newest-first history.
    pub fn state(&self) -> Event {
        self.hub.state()
    }
    /// Current scans, for local PDF export and read-only integrations.
    pub fn scans(&self) -> Vec<Scan> {
        self.hub.session.lock().expect("session").scans.clone()
    }
    /// Stop polling and wait for camera cleanup.
    pub fn stop(&self) {
        self.hub.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.lock().expect("worker").take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        self.stop();
    }
}

fn broadcast(hub: &Arc<Hub>, json: &str) {
    if let Ok(event) = serde_json::from_str::<Event>(json) {
        hub.publish(event);
    }
}

/// How often to ask the platen sensors what they see.
///
/// Fast enough that putting a card down feels answered, slow enough that the
/// two reads it costs are nothing beside a scan.
#[cfg(windows)]
const POLL: std::time::Duration = std::time::Duration::from_millis(200);

/// How long to wait before looking for a scanner again.
#[cfg(windows)]
const RETRY: std::time::Duration = std::time::Duration::from_secs(2);

/// A timing a front end can change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Timing {
    /// How long to wait after a document is detected, before scanning.
    AfterPlacement,
    /// How long to wait after switching lamps, before taking a frame.
    AfterLamp,
}

/// What the service can ask the thread holding the scanner to do.
#[derive(Debug, Clone)]
enum Ask {
    Profile(CaptureSelection),
    /// Scan now, without waiting for the sensors.
    Scan,
    CaptureSide {
        side: passport_scanner_protocol::Side,
        document: Option<String>,
    },
    /// Scan on placement, or do not.
    AutoScan(bool),
    /// Close the document being assembled.
    FinishDocument,
    /// Sound the buzzer on a completed scan, or do not.
    Buzzer(bool),
    /// Measure the platen and keep the result.
    Calibrate,
    /// Discard a calibration measured here.
    ForgetCalibration,
    /// Change a timing, in milliseconds.
    ///
    /// The settings come from the shipped driver's configuration for the
    /// model, which is the right starting point and the wrong last word: how
    /// long to wait after a card is put down depends on who is putting it
    /// down and how quickly they let go.
    Delay(Timing, u64),
    /// Say again what is attached and how things stand.
    ///
    /// A listener only hears what happens while it is connected, so a page
    /// opened after the scanner was found would otherwise never learn of it.
    Describe,
    Select(String),
    Discard(String),
    Side(passport_scanner_protocol::Side),
}

/// Hold the scanner, watch the platen, and do as asked.
///
/// The outer loop is about the device coming and going: unplug the scanner and
/// this reports it, then keeps looking until it is back. The inner loop is the
/// working state - poll, decide, scan - and any device error drops out of it
/// into the outer one, which is the only recovery that makes sense once the
/// thing you are talking to has stopped answering.
#[cfg(windows)]
fn watch(
    listeners: &Arc<Hub>,
    rx: &std::sync::mpsc::Receiver<Ask>,
    mut profile: CaptureSelection,
    preference_file: &std::path::Path,
) {
    use crate::{pages::Album, placement::Action, placement::Trigger};

    broadcast(listeners, r#"{"event":"hello","version":1}"#);

    // These outlive any one device: unplugging a scanner part way through a
    // document should not lose the face already captured.
    let mut album = Album::new();
    let mut auto = listeners.session.lock().expect("session").auto.unwrap_or(true);
    // Off unless asked for: whether a scanner beeps at the person using it is
    // their call, and the answer is usually no.
    let mut buzzer = false;
    let mut announced_missing = false;
    let mut pending_profile_save = false;

    while !listeners.stop.load(Ordering::Acquire) {
        let (mut scanner, model, mut active) = match crate::open_with_selection(profile) {
            Ok(opened) => {
                announced_missing = false;
                opened
            }
            Err(e) => {
                // Say it once, not every two seconds.
                if !announced_missing {
                    announced_missing = true;
                    broadcast(listeners, &device_event(None));
                    broadcast(listeners, &failed("scannerUnavailable", &e.to_string()));
                }
                if drain(
                    listeners,
                    rx,
                    &mut auto,
                    &mut album,
                    &mut profile,
                    &mut pending_profile_save,
                ) {
                    announced_missing = false;
                }
                std::thread::sleep(RETRY);
                continue;
            }
        };

        if pending_profile_save {
            if let Err(e) = crate::preferences::save_to(preference_file, profile) {
                broadcast(
                    listeners,
                    &failed("profileChangeFailed", &format!("Profile was not saved: {e}")),
                );
                drop(scanner);
                drain(
                    listeners,
                    rx,
                    &mut auto,
                    &mut album,
                    &mut profile,
                    &mut pending_profile_save,
                );
                std::thread::sleep(RETRY);
                continue;
            }
            pending_profile_save = false;
        }

        broadcast(listeners, &device_event(Some(model)));

        // Loaded once per device: it is three and a half megabytes and does
        // not change while the scanner is plugged in.
        if active.shading.is_none() {
            broadcast(
                listeners,
                &failed(
                    "noCalibration",
                    "No General flat-field calibration is available for this scanner.",
                ),
            );
        }
        broadcast(listeners, &format!(r#"{{"event":"autoScanChanged","enabled":{auto}}}"#));

        let mut settings = passport_scanner_device::settings::Settings::load(
            passport_scanner_device::protocol::VENDOR_ID,
            model.product_id(),
        );
        broadcast(listeners, &timings_event(&settings, buzzer, profile));
        broadcast(listeners, &calibration_event(&active));

        scanner.set_buzzer(buzzer);
        let mut trigger = Trigger::new();
        if std::mem::take(&mut listeners.session.lock().expect("session").resumed) {
            // A document already on the platen must not be captured again just
            // because the application was restarted to install an update.
            trigger.mark_scanned();
        }
        let mut reported = None;

        'connected: while !listeners.stop.load(Ordering::Acquire) {
            let mut by_hand = false;
            while let Ok(ask) = rx.try_recv() {
                match ask {
                    Ask::Profile(requested) => {
                        match crate::select_capture(&mut scanner, model, requested) {
                            Ok(candidate) => {
                                // No capture can interleave on this device-owning thread.
                                // If saving fails, restore the old device profile before
                                // retaining the old shading. A failed rollback retires it.
                                if let Err(e) =
                                    crate::preferences::save_to(preference_file, requested)
                                {
                                    broadcast(
                                        listeners,
                                        &failed(
                                            "profileChangeFailed",
                                            &format!("Profile was not saved: {e}"),
                                        ),
                                    );
                                    if scanner
                                        .set_capture_profile_with_exposure_percent(
                                            profile.profile(),
                                            profile.exposure_percent(),
                                        )
                                        .is_err()
                                    {
                                        break 'connected;
                                    }
                                } else {
                                    active = candidate;
                                    profile = requested;
                                }
                            }
                            Err(e) => {
                                broadcast(listeners, &failed("profileChangeFailed", &e.to_string()))
                            }
                        }
                        broadcast(listeners, &timings_event(&settings, buzzer, profile));
                        broadcast(listeners, &calibration_event(&active));
                    }
                    Ask::Scan => {
                        by_hand = true;
                        break;
                    }
                    Ask::CaptureSide { side, document } => {
                        if let Some(document) = document {
                            let exists = listeners
                                .session
                                .lock()
                                .expect("session")
                                .scans
                                .iter()
                                .any(|s| s.document == document);
                            if !exists {
                                broadcast(
                                    listeners,
                                    &failed(
                                        "missingDocument",
                                        "That document is no longer in this session.",
                                    ),
                                );
                                continue;
                            }
                            album.select(document);
                        } else {
                            album.finish();
                        }
                        album.set_side(side);
                        by_hand = true;
                        break;
                    }
                    Ask::AutoScan(on) => {
                        auto = on;
                        broadcast(
                            listeners,
                            &format!(r#"{{"event":"autoScanChanged","enabled":{auto}}}"#),
                        );
                    }
                    Ask::FinishDocument => {
                        album.finish();
                        listeners.select(None);
                    }
                    Ask::Select(id) => {
                        if let Some(document) = listeners.document(&id) {
                            album.select(document);
                            listeners.select(Some(id));
                        }
                    }
                    Ask::Discard(id) => {
                        listeners.discard(&id);
                        album.finish();
                    }
                    Ask::Side(side) => album.set_side(side),
                    Ask::Calibrate => {
                        if let Err(e) = crate::calibration::require_general(profile.profile()) {
                            broadcast(listeners, &failed("calibrationRefused", &e.to_string()));
                            continue;
                        }
                        broadcast(
                            listeners,
                            r#"{"event":"scanProgress","stage":"capturing","source":"white"}"#,
                        );
                        match calibrate(&mut scanner, model) {
                            Ok(fresh) => {
                                active.shading = Some(fresh);
                                active.source = Some(crate::calibration::OWN_FILE);
                                broadcast(listeners, &calibration_event(&active));
                            }
                            Err(e) => {
                                broadcast(listeners, &failed("calibrationRefused", &e.to_string()));
                            }
                        }
                    }
                    Ask::ForgetCalibration => {
                        if let Err(e) = crate::calibration::require_general(profile.profile()) {
                            broadcast(listeners, &failed("calibrationRefused", &e.to_string()));
                            continue;
                        }
                        if let Err(e) = crate::calibration::forget(
                            passport_scanner_device::protocol::VENDOR_ID,
                            model.product_id(),
                        ) {
                            broadcast(listeners, &failed("calibrationRefused", &e.to_string()));
                            continue;
                        }
                        match crate::calibration::load_for_profile(
                            passport_scanner_device::protocol::VENDOR_ID,
                            model.product_id(),
                            profile.profile(),
                        ) {
                            Ok(loaded) => active = loaded,
                            Err(e) => {
                                broadcast(listeners, &failed("badCalibration", &e.to_string()));
                                break 'connected;
                            }
                        }
                        broadcast(listeners, &calibration_event(&active));
                    }
                    Ask::Buzzer(on) => {
                        scanner.set_buzzer(on);
                        buzzer = on;
                        broadcast(listeners, &timings_event(&settings, buzzer, profile));
                    }
                    Ask::Delay(timing, ms) => {
                        let wait = std::time::Duration::from_millis(ms);
                        match timing {
                            Timing::AfterPlacement => settings.delays.first = wait,
                            Timing::AfterLamp => {
                                scanner.set_settle(wait);
                                settings.delays.white = wait;
                            }
                        }
                        broadcast(listeners, &timings_event(&settings, buzzer, profile));
                    }
                    Ask::Describe => {
                        listeners.publish_state();
                        broadcast(listeners, &device_event(Some(model)));
                        broadcast(
                            listeners,
                            &format!(r#"{{"event":"autoScanChanged","enabled":{auto}}}"#),
                        );
                        broadcast(listeners, &timings_event(&settings, buzzer, profile));
                        broadcast(listeners, &calibration_event(&active));
                        if let Some(present) = reported {
                            broadcast(
                                listeners,
                                &format!(r#"{{"event":"documentPresence","present":{present}}}"#),
                            );
                        }
                    }
                }
            }

            let Ok(presence) = scanner.document_present() else {
                break;
            };
            let action = trigger.observe(presence);

            if reported != Some(trigger.present()) {
                reported = Some(trigger.present());
                let present = trigger.present();
                broadcast(
                    listeners,
                    &format!(r#"{{"event":"documentPresence","present":{present}}}"#),
                );
            }

            if by_hand {
                // Asked for by hand: the trigger must not fire on the same
                // document again a moment later.
                trigger.mark_scanned();
            } else if action != Action::Scan || !auto {
                std::thread::sleep(POLL);
                continue;
            } else {
                // A card is still settling when it first trips a sensor. The
                // shipped driver waits FirstDelay here - half a second on the
                // reference unit - before committing to a scan.
                std::thread::sleep(settings.delays.first);
            }

            broadcast(listeners, r#"{"event":"scanProgress","stage":"capturing","source":null}"#);
            // Only this thread accesses the scanner. Once acquisition has finished,
            // processing may run alongside sensor reads, never another capture.
            let frames = match scanner.scan(&model.descriptor().sources) {
                Ok(frames) => frames,
                Err(e) => {
                    broadcast(listeners, &failed("scanFailed", &e.to_string()));
                    break;
                }
            };
            listeners.publish(Event::ScanProgress {
                stage: passport_scanner_protocol::Stage::Processing,
                source: None,
            });
            let (processed, sensor_error) = processing_with_observation(
                || process_capture(frames, active.shading.as_ref(), &mut album, listeners),
                || {
                    let presence = scanner.document_present()?;
                    trigger.observe_deferred(presence);
                    if reported != Some(trigger.present()) {
                        reported = Some(trigger.present());
                        listeners.publish(Event::DocumentPresence { present: trigger.present() });
                    }
                    Ok::<_, Error>(())
                },
                POLL,
            );
            // Commands remain queued until this result has been published. In
            // particular, changing sides/documents cannot relabel these frames.
            match processed {
                Ok(Ok(event)) => listeners.publish(event),
                Ok(Err(e)) => {
                    broadcast(listeners, &failed("scanFailed", &e.to_string()));
                    break;
                }
                Err(_) => {
                    broadcast(
                        listeners,
                        &failed("scanFailed", "Image processing stopped unexpectedly."),
                    );
                    break;
                }
            }
            if let Some(error) = sensor_error {
                // A lost sensor must not discard images already acquired. Publish
                // them first, then reconnect before accepting another capture.
                broadcast(listeners, &failed("sensorDisconnected", &error.to_string()));
                break;
            }
        }
    }
}

/// Take a reference capture and build a calibration from it.
#[cfg(windows)]
fn calibrate(
    scanner: &mut crate::OpenScanner,
    model: passport_scanner_device::Model,
) -> Result<passport_scanner_imaging::Shading, Box<dyn std::error::Error>> {
    crate::calibration::require_general(scanner.capture_profile())?;
    // The sensor's own view, uncorrected and uncropped: the correction is
    // built from what the sensor returns, not from what it looks like after
    // the last correction.
    let frames = scanner.scan(&[passport_scanner_core::LightSource::White])?;
    let frame = frames.first().ok_or("the scanner returned no frame")?;
    let image = image::load_from_memory(&frame.data)?.to_rgb8();
    Ok(crate::calibration::measure(
        &image,
        passport_scanner_device::protocol::VENDOR_ID,
        model.product_id(),
    )?)
}

/// Report which calibration is in use and what it says about the platen.
#[cfg(windows)]
fn calibration_event(active: &crate::calibration::Loaded) -> String {
    use passport_scanner_imaging::flatfield;

    let range = active.shading.as_ref().and_then(|shading| {
        let (mut low, mut high) = (f32::MAX, 0f32);
        for y in (0..flatfield::GRID_H).step_by(7) {
            for x in (0..flatfield::GRID_W).step_by(7) {
                if let Some(g) = shading.at(x, y, 1) {
                    let reference = flatfield::TARGET_VISIBLE / g;
                    low = low.min(reference);
                    high = high.max(reference);
                }
            }
        }
        (high > 0.0).then(|| format!("{low:.0} to {high:.0}"))
    });

    serde_json::to_string(&passport_scanner_protocol::Event::CalibrationChanged {
        source: active.source.map(str::to_owned),
        range,
    })
    .unwrap_or_else(|_| r#"{"event":"calibrationChanged","source":null,"range":null}"#.to_owned())
}

/// Report the timings in force, and where they came from.
#[cfg(windows)]
fn timings_event(
    settings: &passport_scanner_device::settings::Settings,
    buzzer: bool,
    profile: CaptureSelection,
) -> String {
    serde_json::json!({"event": "settingsChanged", "settings": {
        "delayAfterPlacement": settings.delays.first.as_millis().to_string(),
        "delayAfterLamp": settings.delays.white.as_millis().to_string(),
        "buzzer": if buzzer { "on" } else { "off" },
        "source": settings.model.as_deref().unwrap_or("built-in defaults"),
        "captureProfile": crate::preferences::profile_name(profile),
        "profileReady": "true",
        "whiteExposurePercent": profile.exposure_percent().to_string(),
    }})
    .to_string()
}

/// Describe what is attached, or that nothing is.
#[cfg(windows)]
fn device_event(model: Option<passport_scanner_device::Model>) -> String {
    match model {
        Some(model) => format!(
            r#"{{"event":"deviceChanged","device":{{"model":"{}","vendorId":{},"productId":{},"serial":null,"sources":["white","infrared"]}}}}"#,
            model.name(),
            passport_scanner_device::protocol::VENDOR_ID,
            model.product_id()
        ),
        None => r#"{"event":"deviceChanged","device":null}"#.to_owned(),
    }
}

/// Apply commands that arrived while no scanner was attached.
///
/// Turning scanning on placement off, or finishing a document, should not be
/// lost because the device happened to be unplugged at the time.
#[cfg(windows)]
fn drain(
    listeners: &Arc<Hub>,
    rx: &std::sync::mpsc::Receiver<Ask>,
    auto: &mut bool,
    album: &mut crate::pages::Album,
    profile: &mut CaptureSelection,
    pending_profile_save: &mut bool,
) -> bool {
    let mut changed = false;
    while let Ok(ask) = rx.try_recv() {
        match ask {
            Ask::Profile(requested) => {
                *profile = requested;
                *pending_profile_save = true;
                changed = true;
                broadcast(listeners, &serde_json::json!({"event": "settingsChanged", "settings": {
                    "captureProfile": crate::preferences::profile_name(*profile), "profileReady": "false"
                }}).to_string());
            }
            Ask::AutoScan(on) => *auto = on,
            Ask::FinishDocument => {
                album.finish();
                listeners.select(None);
            }
            Ask::Select(id) => {
                if let Some(document) = listeners.document(&id) {
                    album.select(document);
                    listeners.select(Some(id));
                }
            }
            Ask::Discard(id) => {
                listeners.discard(&id);
                album.finish();
            }
            Ask::Side(side) => album.set_side(side),
            // Nothing to apply these to until a device is back.
            Ask::Delay(..) | Ask::Buzzer(_) | Ask::Calibrate | Ask::ForgetCalibration => {}
            // A page opened while the scanner is unplugged still deserves an
            // answer, and the answer is that there is nothing attached.
            Ask::Describe => {
                listeners.publish_state();
                broadcast(listeners, &device_event(None));
                broadcast(listeners, &format!(r#"{{"event":"autoScanChanged","enabled":{auto}}}"#));
                broadcast(listeners, &serde_json::json!({"event": "settingsChanged", "settings": {
                    "captureProfile": crate::preferences::profile_name(*profile), "profileReady": "false"
                }}).to_string());
                broadcast(
                    listeners,
                    r#"{"event":"calibrationChanged","source":null,"range":null}"#,
                );
            }
            // Nothing to scan with.
            Ask::Scan | Ask::CaptureSide { .. } => {
                broadcast(listeners, &failed("scannerUnavailable", "No scanner is connected."))
            }
        }
    }
    changed
}

fn handle_command(text: &str, listeners: &Arc<Hub>, asks: &Sender<Ask>) {
    use passport_scanner_protocol::Command;

    let Ok(command) = serde_json::from_str::<Command>(text) else {
        // An unreadable command is the front end's problem, but a silent
        // failure would be ours.
        broadcast(listeners, &failed("badCommand", "That command was not understood."));
        return;
    };

    let ask = match command {
        Command::StartScan => Some(Ask::Scan),
        Command::CaptureSide { side, document } => Some(Ask::CaptureSide { side, document }),
        Command::SelectScan { id } => Some(Ask::Select(id)),
        Command::Discard { id } => Some(Ask::Discard(id)),
        Command::SetSetting { key, value } if key == "scanSide" => match value.as_str() {
            "front" => Some(Ask::Side(passport_scanner_protocol::Side::Front)),
            "back" => Some(Ask::Side(passport_scanner_protocol::Side::Back)),
            _ => {
                broadcast(listeners, &failed("badSetting", "scanSide must be front or back."));
                return;
            }
        },
        Command::SetAutoScan { enabled } => Some(Ask::AutoScan(enabled)),
        Command::FinishDocument => Some(Ask::FinishDocument),
        Command::GetState => Some(Ask::Describe),
        Command::Calibrate => Some(Ask::Calibrate),
        Command::ForgetCalibration => Some(Ask::ForgetCalibration),
        Command::SetSetting { key, value } if key == "captureProfile" => {
            match crate::preferences::parse_profile(&value) {
                Ok(profile) => Some(Ask::Profile(profile)),
                Err(e) => {
                    broadcast(listeners, &failed("profileChangeFailed", &e.to_string()));
                    return;
                }
            }
        }
        Command::SetSetting { key, value } if key == "buzzer" => {
            Some(Ask::Buzzer(matches!(value.as_str(), "on" | "true" | "1")))
        }
        Command::SetSetting { key, value } => {
            let timing = match key.as_str() {
                "delayAfterPlacement" => Timing::AfterPlacement,
                "delayAfterLamp" => Timing::AfterLamp,
                _ => return,
            };
            match value.trim().parse::<u64>() {
                // An upper bound so a typo cannot wedge the scanner for an
                // hour with no way back short of restarting it.
                Ok(ms) if ms <= 10_000 => Some(Ask::Delay(timing, ms)),
                _ => {
                    broadcast(
                        listeners,
                        &failed(
                            "badSetting",
                            "A delay must be a whole number of milliseconds, up to 10000.",
                        ),
                    );
                    return;
                }
            }
        }
        // Accept has no persistence action; history is held for this session.
        _ => None,
    };
    if let Some(ask) = ask {
        let _ = asks.send(ask);
    }
}

/// Run exactly one processing job while its caller continues observing sensors.
/// A sensor error stops further reads, but the acquired images finish processing.
#[cfg(any(windows, test))]
fn processing_with_observation<T: Send, E>(
    process: impl FnOnce() -> T + Send,
    mut observe: impl FnMut() -> Result<(), E>,
    interval: std::time::Duration,
) -> (std::thread::Result<T>, Option<E>) {
    std::thread::scope(|scope| {
        let (done, completion) = channel();
        let job = scope.spawn(move || {
            let result = process();
            let _ = done.send(());
            result
        });
        let mut sensor_error = None;
        loop {
            match completion.recv_timeout(interval) {
                Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if let Err(error) = observe() {
                        sensor_error = Some(error);
                        // No further hardware access after a failed sensor read.
                        // Joining still preserves the processing result.
                        break;
                    }
                }
            }
        }
        (job.join(), sensor_error)
    })
}

/// Enhance acquired frames and turn them into a completed-scan event.
#[cfg(windows)]
fn process_capture(
    mut frames: Vec<passport_scanner_core::Frame>,
    shading: Option<&passport_scanner_imaging::Shading>,
    album: &mut crate::pages::Album,
    listeners: &Arc<Hub>,
) -> Result<Event, Box<dyn std::error::Error + Send + Sync>> {
    use passport_scanner_protocol::{Field, Image, Scan, Source, Span};

    crate::enhance(&mut frames, shading)?;

    broadcast(listeners, r#"{"event":"scanProgress","stage":"recognising","source":null}"#);
    let recognized = match crate::read_document_oriented(&frames) {
        Ok(document) => document,
        Err(_) => {
            // Keep usable captures even when OCR fails. Never expose guessed fields.
            broadcast(
                listeners,
                &failed(
                    "recognitionFailed",
                    "Images captured, but the MRZ could not be read. Check the back side and retry; all four orientations were attempted.",
                ),
            );
            passport_scanner_recognize::Recognition::default()
        }
    };
    if let Some(angle) = recognized.rotation {
        if let Err(error) = crate::orientation::orient_frames(&mut frames, angle) {
            // Orientation is a presentation improvement; keep the acquired
            // images if their metadata cannot safely be updated.
            broadcast(listeners, &failed("orientationFailed", &error.to_string()));
        }
    }
    let doc = recognized.document;
    let images: Vec<Image> = frames
        .iter()
        .map(|f| {
            let (width, height) =
                crate::orientation::display_dimensions(f).unwrap_or((f.width, f.height));
            Image {
                source: match f.source {
                    passport_scanner_core::LightSource::White => Source::White,
                    passport_scanner_core::LightSource::Infrared => Source::Infrared,
                    passport_scanner_core::LightSource::Ultraviolet => Source::Ultraviolet,
                },
                url: format!("data:image/jpeg;base64,{}", base64(&f.data)),
                width,
                height,
            }
        })
        .collect();
    let fields: Vec<Field> = doc
        .fields
        .values()
        .filter_map(|f| {
            Some(Field {
                kind: map_kind(f.kind)?,
                value: f.value.clone(),
                check: f.checksum_ok,
                span: None::<Span>,
            })
        })
        .collect();

    let mrz: Vec<String> = doc
        .get(passport_scanner_core::FieldKind::MrzLine)
        .map(|f| f.value.lines().map(str::to_owned).collect())
        .unwrap_or_default();

    let at = stamp();
    let placed = album.place(format!("d{at}"));

    let scan = Scan {
        id: format!("s{at}"),
        document: placed.document,
        side: placed.side,
        at: time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)?,
        format: match (mrz.len(), mrz.first().map(String::len)) {
            (3, Some(30)) => Some("TD1".into()),
            (2, Some(36)) => Some("TD2".into()),
            (2, Some(44)) => Some("TD3".into()),
            _ => None,
        },
        mrz,
        mrz_check: doc
            .get(passport_scanner_core::FieldKind::MrzLine)
            .and_then(|field| field.checksum_ok),
        fields,
        images,
    };
    Ok(Event::ScanCompleted { scan: Box::new(scan) })
}

/// A failure event, with the message made safe to put in one.
fn failed(code: &str, message: &str) -> String {
    serde_json::to_string(&passport_scanner_protocol::Event::Failed {
        code: code.to_owned(),
        message: message.to_owned(),
    })
    .unwrap_or_else(|_| {
        // Serialising two strings cannot realistically fail, but a front end
        // waiting on an event it never gets is worse than a vague one.
        r#"{"event":"failed","code":"internal","message":"Something went wrong."}"#.to_owned()
    })
}

#[cfg(windows)]
fn map_kind(k: passport_scanner_core::FieldKind) -> Option<passport_scanner_protocol::FieldKind> {
    use passport_scanner_core::FieldKind as C;
    use passport_scanner_protocol::FieldKind as P;
    Some(match k {
        C::GivenName => P::GivenName,
        C::Surname => P::Surname,
        C::DocumentNumber => P::DocumentNumber,
        C::Nationality => P::Nationality,
        C::IssuingState => P::IssuingState,
        C::DateOfBirth => P::DateOfBirth,
        C::DateOfExpiry => P::DateOfExpiry,
        C::Sex => P::Sex,
        C::DocumentType => P::DocumentType,
        C::OptionalData => P::OptionalData,
        // The raw zone lines travel in their own field of the message, and
        // FieldKind is non-exhaustive, so anything unmapped is simply dropped
        // rather than guessed at.
        _ => return None,
    })
}

/// Nanoseconds since the epoch, for distinct capture identifiers.
#[cfg(windows)]
fn stamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default()
}

/// Minimal base64, so a frame can ride in a data URL without another crate.
#[cfg(windows)]
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - i * 6)) & 0x3F) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn profile_command_is_strict_and_queues_the_named_selection() {
        let listeners = Arc::new(Hub::default());
        let (tx, rx) = channel();
        handle_command(
            r#"{"command":"setSetting","key":"captureProfile","value":"identity-bright"}"#,
            &listeners,
            &tx,
        );
        assert!(matches!(rx.try_recv(), Ok(Ask::Profile(CaptureSelection::IdentityBright))));
        handle_command(
            r#"{"command":"setSetting","key":"captureProfile","value":"unknown"}"#,
            &listeners,
            &tx,
        );
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn unavailable_identity_can_request_general_without_claiming_ready() {
        let (events, received) = channel();
        let listeners = Arc::new(Hub::with_listener(events));
        let (tx, rx) = channel();
        tx.send(Ask::Profile(CaptureSelection::General)).unwrap();
        tx.send(Ask::Describe).unwrap();
        let mut profile = CaptureSelection::IdentityBright;
        let mut pending_save = false;
        assert!(drain(
            &listeners,
            &rx,
            &mut true,
            &mut crate::pages::Album::new(),
            &mut profile,
            &mut pending_save
        ));
        assert_eq!(profile, CaptureSelection::General);
        assert!(pending_save);
        let events: Vec<serde_json::Value> =
            received.try_iter().map(|event| serde_json::from_str(&event).unwrap()).collect();
        let settings = events.iter().filter(|e| e["event"] == "settingsChanged");
        assert!(settings.clone().count() > 0);
        for event in settings {
            assert_eq!(event["settings"]["captureProfile"], "general");
            assert_eq!(event["settings"]["profileReady"], "false");
        }
    }

    #[test]
    fn describe_payload_reports_loaded_calibration_and_selection() {
        let loaded = crate::calibration::Loaded {
            profile: passport_scanner_device::CaptureProfile::Identity,
            source: Some(crate::calibration::IDENTITY_FILE),
            shading: None,
        };
        let event: serde_json::Value = serde_json::from_str(&calibration_event(&loaded)).unwrap();
        assert_eq!(event["source"], crate::calibration::IDENTITY_FILE);
        let settings: serde_json::Value = serde_json::from_str(&timings_event(
            &passport_scanner_device::settings::Settings::default(),
            false,
            CaptureSelection::IdentityBright,
        ))
        .unwrap();
        assert_eq!(settings["settings"]["captureProfile"], "identity-bright");
        assert_eq!(settings["settings"]["whiteExposurePercent"], "120");
    }
}
