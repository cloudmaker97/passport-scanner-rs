//! The Passport Scanner interface, shared by the native window and the browser.
//!
//! It speaks only the wire contract in `passport-scanner-protocol`, through a
//! [`Backend`]. Platform specifics - owning the scanner, saving files, browser
//! storage - stay in the thin clients that construct [`App`].

pub mod backend;
pub mod i18n;
pub mod mock;
mod model;
#[cfg(test)]
mod orientation_tests;
pub mod pdf_name;
pub mod preferences;
mod view;

pub use backend::{Backend, ExportOutcome, ExportRequest, Inbound, LocalServices, Offline};
pub use i18n::Locale;
pub use preferences::{Appearance, Memory, Preferences, Store};

use base64::Engine as _;
use i18n::Strings;
use iced::widget::image;
use iced::{Subscription, Task};
use model::{Document, Documents};
use passport_scanner_protocol::{Command, Device, Event, Scan, Side, Source, Stage};
use std::{collections::HashMap, time::Duration};

pub const TITLE: &str = "Passport Scanner";
/// Monospace face for the MRZ; browsers expose no system fonts to the canvas.
pub const MONO_FONT: iced::Font = iced::Font::with_name("Fira Mono");
const MONO_FONT_BYTES: &[u8] = include_bytes!("../assets/FiraMono-Medium.ttf");
pub const WEB_URL: &str = "http://127.0.0.1:8733";
pub const MCP_URL: &str = "http://127.0.0.1:8735/mcp";

/// Renderer settings both clients use.
pub fn settings() -> iced::Settings {
    iced::Settings {
        default_text_size: iced::Pixels(14.),
        fonts: vec![MONO_FONT_BYTES.into()],
        ..Default::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    General,
    Identity,
    Bright,
}

impl Profile {
    pub const ALL: [Self; 3] = [Self::General, Self::Identity, Self::Bright];
    pub fn key(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Identity => "identity",
            Self::Bright => "identity-bright",
        }
    }
    pub fn from_key(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == value)
    }
    fn label(self, t: &Strings) -> &'static str {
        match self {
            Self::General => t.profile_general,
            Self::Identity => t.profile_identity,
            Self::Bright => t.profile_bright,
        }
    }
}

/// What the status bar says. Kept symbolic so a language switch relabels it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Connecting,
    NotConnected,
    ScannerDisconnected,
    CapturingSide(Side),
    LoadingProfile,
    Calibrating,
    Ready,
    CalibrationMissing,
    Stage(Stage),
    ScanComplete,
    ActionFailed,
    #[cfg(feature = "render-smoke")]
    Preview,
}

impl Status {
    fn label(self, t: &Strings) -> String {
        match self {
            Self::Connecting => t.connecting.into(),
            Self::NotConnected => t.not_connected.into(),
            Self::ScannerDisconnected => t.scanner_disconnected.into(),
            Self::CapturingSide(side) => t.capturing(side),
            Self::LoadingProfile => t.loading_profile.into(),
            Self::Calibrating => t.calibrating.into(),
            Self::Ready => t.ready.into(),
            Self::CalibrationMissing => t.calibration_missing.into(),
            Self::Stage(stage) => match stage {
                Stage::AwaitingDocument => t.place_document,
                Stage::Capturing => t.stage_capturing,
                Stage::Processing => t.stage_processing,
                Stage::Recognising => t.stage_recognising,
                Stage::Settling => t.stage_settling,
                _ => t.stage_scanning,
            }
            .into(),
            Self::ScanComplete => t.scan_complete.into(),
            Self::ActionFailed => t.action_failed.into(),
            #[cfg(feature = "render-smoke")]
            Self::Preview => t.preview.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    Reconnect,
    Capture(Side),
    NewDocument,
    Select(String),
    Profile(Profile),
    Auto(bool),
    Buzzer(bool),
    Web(bool),
    Mcp(bool),
    Calibrate,
    ForgetCalibration,
    Export,
    /// Right-click on a history entry: open its menu, or close with `None`.
    HistoryMenu(Option<String>),
    SaveDocumentAs(String),
    DeleteDocument(String),
    Exported(Result<ExportOutcome, String>),
    PreviewSide(Side),
    PreviewSource(Source),
    OpenSettings,
    CloseSettings,
    Appearance(Appearance),
    Locale(Locale),
    CopyMrz,
    CopyWebUrl,
    CopyMcpUrl,
    ChooseFolder,
    FolderChosen(Option<String>),
    ResetFolder,
    DismissNotice,
    #[cfg(feature = "render-smoke")]
    Screenshot(iced::window::Screenshot),
}

#[derive(Debug)]
pub struct App {
    // Dropping the native backend stops its scanner worker and child services,
    // never unrelated user processes.
    backend: Box<dyn Backend>,
    store: Box<dyn Store>,
    documents: Documents,
    selected: Option<String>,
    images: HashMap<String, image::Handle>,
    device: Option<Device>,
    profile: Profile,
    profile_ready: bool,
    profile_pending: bool,
    calibration: Option<String>,
    busy: bool,
    status: Status,
    auto: bool,
    buzzer: bool,
    web: bool,
    mcp: bool,
    present: bool,
    exporting: bool,
    /// Complete documents waiting to be saved by themselves, oldest first.
    pending_saves: Vec<String>,
    /// The document whose automatic save is running.
    auto_saving: Option<String>,
    /// Where each document was saved by itself, so a rescan replaces that file.
    auto_saved: HashMap<String, String>,
    notice: Option<(String, bool)>,
    preview_side: Side,
    preview_source: Source,
    settings_open: bool,
    /// The history entry whose context menu is open.
    history_menu: Option<String>,
    appearance: Appearance,
    locale: Locale,
    pdf_folder: Option<String>,
    #[cfg(feature = "render-smoke")]
    smoke_ticks: u8,
    #[cfg(feature = "render-smoke")]
    smoke_path: Option<std::path::PathBuf>,
    #[cfg(feature = "render-smoke")]
    offline_smoke: bool,
}

impl App {
    pub fn new(backend: Box<dyn Backend>, store: Box<dyn Store>) -> Self {
        let mut state = Self::unconnected(backend, store);
        match state.store.load() {
            Ok(preferences) => {
                state.appearance = preferences.appearance;
                if let Some(locale) = preferences.locale {
                    state.locale = locale;
                }
                state.pdf_folder = preferences.pdf_folder;
            }
            Err(error) => state.fail(format!("{}: {error}", state.t().settings_not_loaded)),
        }
        // The opt-in renderer helper never opens a scanner, even if the
        // supplied session is missing or invalid.
        #[cfg(feature = "render-smoke")]
        if let Some(path) = std::env::var_os("PASSPORT_SCANNER_UI_SMOKE_SESSION") {
            if let Err(error) = state.load_smoke_session(std::path::Path::new(&path)) {
                state.fail(format!("Render: {error}"));
            }
        } else {
            state.connect();
        }
        #[cfg(not(feature = "render-smoke"))]
        state.connect();
        #[cfg(feature = "render-smoke")]
        if state.smoke_path.is_some() {
            state.settings_open =
                std::env::var("PASSPORT_SCANNER_UI_SMOKE_VIEW").as_deref() == Ok("settings");
            match std::env::var("PASSPORT_SCANNER_UI_SMOKE_THEME").as_deref() {
                Ok("light") => state.appearance = Appearance::Light,
                Ok("dark") => state.appearance = Appearance::Dark,
                _ => {}
            }
            if let Ok(locale) = std::env::var("PASSPORT_SCANNER_UI_SMOKE_LOCALE") {
                state.locale = Locale::detect(&locale);
            }
        }
        state
    }

    fn unconnected(backend: Box<dyn Backend>, store: Box<dyn Store>) -> Self {
        let profile =
            backend.saved_profile().and_then(|p| Profile::from_key(&p)).unwrap_or(Profile::General);
        let locale = Locale::detect(&store.system_locale().unwrap_or_default());
        Self {
            backend,
            store,
            documents: Documents::default(),
            selected: None,
            images: HashMap::new(),
            device: None,
            profile,
            profile_ready: false,
            profile_pending: false,
            calibration: None,
            busy: false,
            status: Status::Connecting,
            auto: false,
            buzzer: false,
            web: false,
            mcp: false,
            present: false,
            exporting: false,
            pending_saves: Vec::new(),
            auto_saving: None,
            auto_saved: HashMap::new(),
            notice: None,
            preview_side: Side::Front,
            preview_source: Source::White,
            settings_open: false,
            history_menu: None,
            appearance: Appearance::default(),
            locale,
            pdf_folder: None,
            #[cfg(feature = "render-smoke")]
            smoke_ticks: 0,
            #[cfg(feature = "render-smoke")]
            smoke_path: std::env::var_os("PASSPORT_SCANNER_UI_SMOKE_SCREENSHOT")
                .map(std::path::PathBuf::from),
            #[cfg(feature = "render-smoke")]
            offline_smoke: std::env::var_os("PASSPORT_SCANNER_UI_SMOKE_SESSION").is_some(),
        }
    }

    #[cfg(feature = "render-smoke")]
    fn load_smoke_session(&mut self, path: &std::path::Path) -> std::io::Result<()> {
        let events: Vec<Event> = serde_json::from_slice(&std::fs::read(path)?)?;
        if !events.iter().any(|event| matches!(event, Event::State { .. })) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "no session state"));
        }
        for event in events {
            self.receive(event);
        }
        self.status = Status::Preview;
        Ok(())
    }

    fn t(&self) -> &'static Strings {
        self.locale.strings()
    }

    fn connect(&mut self) {
        #[cfg(feature = "render-smoke")]
        if self.offline_smoke {
            return;
        }
        if self.backend.connected() {
            return;
        }
        match self.backend.connect() {
            Ok(events) => {
                for event in events {
                    self.receive(event);
                }
                if self.backend.connected()
                    && let Err(e) = self.backend.send(Command::GetState)
                {
                    self.fail(e);
                }
                self.refresh_services();
            }
            Err(e) => {
                self.status = Status::NotConnected;
                if !e.is_empty() {
                    self.fail(e);
                }
            }
        }
    }

    fn refresh_services(&mut self) {
        if let Some(services) = self.backend.services() {
            self.web = services.web_running();
            self.mcp = services.mcp_running();
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::time::every(Duration::from_millis(100)).map(|_| Message::Tick),
            iced::event::listen_with(|event, _, _| match event {
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                    ..
                }) => Some(Message::CloseSettings),
                _ => None,
            }),
        ])
    }

    fn fail(&mut self, message: impl Into<String>) {
        self.notice = Some((message.into(), true));
    }

    fn send(&mut self, command: Command) -> bool {
        let result = if self.backend.connected() {
            self.backend.send(command)
        } else {
            Err(self.t().service_not_started.to_owned())
        };
        match result {
            Ok(()) => true,
            Err(e) => {
                self.fail(e);
                false
            }
        }
    }

    fn save_preferences(&mut self) {
        let preferences = Preferences {
            appearance: self.appearance,
            locale: Some(self.locale),
            pdf_folder: self.pdf_folder.clone(),
        };
        if let Err(error) = self.store.save(preferences) {
            self.fail(format!("{}: {error}", self.t().settings_not_saved));
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => {
                for inbound in self.backend.poll() {
                    match inbound {
                        Inbound::Event(event) => self.receive(event),
                        Inbound::Unreadable => self.fail(self.t().unreadable_event),
                        Inbound::Connection(false) => {
                            self.busy = false;
                            self.status = Status::NotConnected;
                            self.fail(self.t().connection_lost);
                        }
                        Inbound::Connection(true) => {
                            if self
                                .notice
                                .as_ref()
                                .is_some_and(|(n, _)| n == self.t().connection_lost)
                            {
                                self.notice = None;
                            }
                        }
                    }
                }
                self.refresh_services();
                #[cfg(feature = "render-smoke")]
                if self.smoke_path.is_some() {
                    self.smoke_ticks = self.smoke_ticks.saturating_add(1);
                    if self.smoke_ticks == 30 {
                        return iced::window::latest()
                            .and_then(iced::window::screenshot)
                            .map(Message::Screenshot);
                    }
                }
                return self.auto_save();
            }
            Message::Reconnect => self.connect(),
            Message::Capture(side) if self.can_scan() => {
                if self.send(self.capture_command(side)) {
                    self.busy = true;
                    self.status = Status::CapturingSide(side);
                }
            }
            Message::NewDocument if !self.busy => {
                if self.send(Command::FinishDocument) {
                    self.selected = None;
                    self.preview_side = Side::Front;
                    self.notice = None;
                }
            }
            Message::Select(id) => {
                self.history_menu = None;
                let scan_id = self
                    .documents
                    .get(&id)
                    .and_then(Document::representative_scan)
                    .map(|s| (s.id.clone(), s.side));
                self.selected = Some(id);
                if let Some((id, side)) = scan_id {
                    self.preview_side = side;
                    self.send(Command::SelectScan { id });
                }
            }
            Message::Profile(profile) if !self.busy && !self.profile_pending => {
                if self.send(Command::SetSetting {
                    key: "captureProfile".into(),
                    value: profile.key().into(),
                }) {
                    self.profile_pending = true;
                    self.status = Status::LoadingProfile;
                }
            }
            Message::Auto(enabled) => {
                self.send(Command::SetAutoScan { enabled });
            }
            Message::Buzzer(enabled) => {
                self.send(Command::SetSetting {
                    key: "buzzer".into(),
                    value: if enabled { "on" } else { "off" }.into(),
                });
            }
            Message::Web(enabled) => {
                if let Some(Err(e)) = self.backend.services().map(|s| s.set_web_enabled(enabled)) {
                    self.fail(e);
                }
                self.refresh_services();
            }
            Message::Mcp(enabled) => {
                if let Some(Err(e)) = self.backend.services().map(|s| s.set_mcp_enabled(enabled)) {
                    self.fail(e);
                }
                self.refresh_services();
            }
            Message::Calibrate if self.can_calibrate() => {
                if self.send(Command::Calibrate) {
                    self.busy = true;
                    self.status = Status::Calibrating;
                }
            }
            Message::ForgetCalibration if self.can_calibrate() => {
                self.send(Command::ForgetCalibration);
            }
            Message::Export => {
                if let Some(id) = self.selected.clone() {
                    return self.export(id, self.pdf_folder.clone(), true, None);
                }
            }
            Message::HistoryMenu(id) => self.history_menu = id,
            Message::SaveDocumentAs(id) => {
                self.history_menu = None;
                // Always ask, even where a permanent folder is set.
                return self.export(id, None, true, None);
            }
            Message::DeleteDocument(id) => {
                self.history_menu = None;
                let scan = self
                    .documents
                    .get(&id)
                    .and_then(Document::representative_scan)
                    .map(|s| s.id.clone());
                if let Some(scan) = scan {
                    self.send(Command::Discard { id: scan });
                }
            }
            Message::Exported(result) => {
                self.exporting = false;
                let auto = self.auto_saving.take();
                let t = self.t();
                match result {
                    Ok(ExportOutcome::Saved(path)) => {
                        self.notice = Some((format!("{}: {path}", t.pdf_saved), false));
                        if let Some(id) = auto {
                            self.auto_saved.insert(id, path);
                        }
                    }
                    Ok(ExportOutcome::Downloaded) => {
                        self.notice = Some((t.pdf_downloaded.into(), false));
                    }
                    Ok(ExportOutcome::Cancelled) => {}
                    Ok(ExportOutcome::Unavailable) => self.fail(t.pdf_unavailable),
                    Err(e) => self.fail(e),
                }
                return self.auto_save();
            }
            Message::PreviewSide(side) => self.preview_side = side,
            Message::PreviewSource(source) => self.preview_source = source,
            Message::OpenSettings => self.settings_open = true,
            Message::CloseSettings => {
                self.settings_open = false;
                self.history_menu = None;
            }
            Message::Appearance(appearance) => {
                self.appearance = appearance;
                self.save_preferences();
            }
            Message::Locale(locale) => {
                self.locale = locale;
                self.save_preferences();
            }
            Message::CopyMrz => {
                if let Some(lines) = self
                    .selected_document()
                    .and_then(Document::recognised)
                    .map(|scan| scan.mrz.join("\n"))
                    .filter(|lines| !lines.is_empty())
                {
                    self.notice = Some((self.t().mrz_copied.into(), false));
                    return self.backend.copy_text(lines);
                }
            }
            Message::CopyWebUrl => return self.backend.copy_text(WEB_URL.into()),
            Message::CopyMcpUrl => return self.backend.copy_text(MCP_URL.into()),
            Message::ChooseFolder => {
                return self.backend.choose_folder(self.t().choose_folder_title.into());
            }
            Message::FolderChosen(Some(folder)) => {
                self.pdf_folder = Some(folder);
                self.save_preferences();
            }
            Message::FolderChosen(None) => {}
            Message::ResetFolder => {
                self.pdf_folder = None;
                self.save_preferences();
            }
            Message::DismissNotice => self.notice = None,
            #[cfg(feature = "render-smoke")]
            Message::Screenshot(screenshot) => {
                if let Some(path) = self.smoke_path.take() {
                    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                        if let Some(folder) = path.parent() {
                            std::fs::create_dir_all(folder)?;
                        }
                        ::image::save_buffer(
                            &path,
                            &screenshot.rgba,
                            screenshot.size.width,
                            screenshot.size.height,
                            ::image::ColorType::Rgba8,
                        )?;
                        Ok(())
                    })();
                    if let Err(error) = result {
                        self.fail(format!("Render: {error}"));
                    }
                    if std::env::var("PASSPORT_SCANNER_UI_SMOKE_CLOSE").as_deref() == Ok("1") {
                        return iced::window::latest().and_then(iced::window::close);
                    }
                }
            }
            _ => {}
        }
        Task::none()
    }

    /// Export one complete document; without a folder, `ask` decides whether a
    /// dialog asks where to save. `path` replaces an earlier export instead.
    fn export(
        &mut self,
        id: String,
        folder: Option<String>,
        ask: bool,
        path: Option<String>,
    ) -> Task<Message> {
        if self.exporting || self.documents.get(&id).is_none_or(|d| d.image_count() != 4) {
            return Task::none();
        }
        let t = self.t();
        self.exporting = true;
        self.backend.export_pdf(
            id,
            ExportRequest {
                title: t.save_dialog_title.into(),
                filter: t.pdf_filter.into(),
                folder,
                ask,
                path,
            },
        )
    }

    /// Save the next complete document by itself, over its earlier automatic
    /// save if there is one. Waits while another export runs.
    fn auto_save(&mut self) -> Task<Message> {
        if self.exporting || self.pending_saves.is_empty() {
            return Task::none();
        }
        let id = self.pending_saves.remove(0);
        let path = self.auto_saved.get(&id).cloned();
        let task = self.export(id.clone(), self.pdf_folder.clone(), false, path);
        if self.exporting {
            self.auto_saving = Some(id);
        }
        task
    }

    fn ready_status(&self) -> Status {
        if self.profile_ready { Status::Ready } else { Status::CalibrationMissing }
    }

    fn receive(&mut self, event: Event) {
        match event {
            Event::State { device, scans, selected } => {
                self.device = device;
                if self.status == Status::Connecting || self.status == Status::NotConnected {
                    self.status = self.ready_status();
                }
                let selected_document = selected
                    .as_ref()
                    .and_then(|id| scans.iter().find(|s| &s.id == id))
                    .map(|s| s.document.clone());
                if self.selected != selected_document {
                    self.preview_side = selected
                        .as_ref()
                        .and_then(|id| scans.iter().find(|scan| &scan.id == id))
                        .map(|scan| scan.side)
                        .unwrap_or(Side::Front);
                }
                // Compare against the existing model before replacing it. An
                // unchanged Iced handle also preserves its decoded renderer image.
                let previous: HashMap<&str, &Scan> = self
                    .documents
                    .0
                    .iter()
                    .flat_map(|document| document.front.iter().chain(document.back.iter()))
                    .map(|scan| (scan.id.as_str(), scan))
                    .collect();
                let mut documents = Documents::default();
                for scan in scans.into_iter().rev() {
                    cache_images(&mut self.images, &scan, previous.get(scan.id.as_str()).copied());
                    documents.upsert(scan);
                }
                self.documents = documents;
                self.prune_images();
                self.selected = selected_document;
            }
            Event::DeviceChanged { device } => {
                self.device = device;
                if self.device.is_none() {
                    self.profile_ready = false;
                    self.busy = false;
                    self.status = Status::ScannerDisconnected;
                }
            }
            Event::SettingsChanged { settings } => {
                if let Some(profile) =
                    settings.get("captureProfile").and_then(|s| Profile::from_key(s))
                {
                    self.profile = profile;
                    self.profile_pending = false;
                }
                if let Some(ready) = settings.get("profileReady") {
                    self.profile_ready = ready == "true";
                }
                if let Some(buzzer) = settings.get("buzzer") {
                    self.buzzer = buzzer == "on";
                }
                if !self.busy {
                    self.status = self.ready_status();
                }
            }
            Event::CalibrationChanged { source, .. } => {
                self.calibration = source;
                self.busy = false;
                self.status = self.ready_status();
            }
            Event::AutoScanChanged { enabled } => self.auto = enabled,
            Event::DocumentPresence { present } => self.present = present,
            Event::ScanProgress { stage, .. } => {
                self.busy = stage != Stage::AwaitingDocument;
                self.status = Status::Stage(stage);
            }
            Event::ScanCompleted { scan } => {
                self.busy = false;
                self.status = Status::ScanComplete;
                self.notice = None;
                self.selected = Some(scan.document.clone());
                self.preview_side = scan.side;
                self.cache_scan(&scan);
                let id = scan.document.clone();
                self.documents.upsert(*scan);
                self.prune_images();
                if self.backend.saves_automatically()
                    && self.documents.get(&id).is_some_and(|d| d.image_count() == 4)
                    && !self.pending_saves.contains(&id)
                {
                    self.pending_saves.push(id);
                }
            }
            Event::Failed { code, message } => {
                self.busy = false;
                self.profile_pending = false;
                if matches!(
                    code.as_str(),
                    "scannerUnavailable" | "deviceNotFound" | "badCalibration"
                ) {
                    self.profile_ready = false;
                }
                self.status = Status::ActionFailed;
                self.fail(self.t().failure(&code, &message));
            }
            _ => {}
        }
    }

    fn cache_scan(&mut self, scan: &Scan) {
        let previous = self
            .documents
            .0
            .iter()
            .flat_map(|document| document.front.iter().chain(document.back.iter()))
            .find(|previous| previous.id == scan.id);
        cache_images(&mut self.images, scan, previous);
    }

    fn prune_images(&mut self) {
        let live: std::collections::HashSet<String> = self
            .documents
            .0
            .iter()
            .flat_map(|document| document.front.iter().chain(document.back.iter()))
            .flat_map(|scan| (0..scan.images.len()).map(|index| format!("{}:{index}", scan.id)))
            .collect();
        self.images.retain(|key, _| live.contains(key));
    }

    fn selected_document(&self) -> Option<&Document> {
        self.selected.as_deref().and_then(|id| self.documents.get(id))
    }

    fn capture_command(&self, side: Side) -> Command {
        Command::CaptureSide {
            side,
            document: self.selected_document().map(|document| document.id.clone()),
        }
    }

    fn can_scan(&self) -> bool {
        self.backend.connected()
            && self.device.is_some()
            && self.profile_ready
            && !self.profile_pending
            && !self.busy
    }

    fn can_calibrate(&self) -> bool {
        self.can_scan() && self.profile == Profile::General
    }

    fn image_handle(
        &self,
        document: &Document,
        side: Side,
        source: Source,
    ) -> Option<&image::Handle> {
        let scan = document.page(side)?;
        let index = scan.images.iter().position(|i| i.source == source)?;
        self.images.get(&format!("{}:{index}", scan.id))
    }
}

fn decode_image(url: &str) -> Option<Vec<u8>> {
    let (header, bytes) = url.split_once(',')?;
    if !matches!(header, "data:image/jpeg;base64" | "data:image/png;base64") {
        return None;
    }
    base64::engine::general_purpose::STANDARD.decode(bytes).ok()
}

fn cache_images(cache: &mut HashMap<String, image::Handle>, scan: &Scan, previous: Option<&Scan>) {
    for (index, source) in scan.images.iter().enumerate() {
        let key = format!("{}:{index}", scan.id);
        if previous.and_then(|scan| scan.images.get(index)) == Some(source)
            && cache.contains_key(&key)
        {
            continue;
        }
        if let Some(bytes) = decode_image(&source.url) {
            // Iced's renderer applies EXIF orientation while loading these
            // bytes. Keep this lazy: eagerly rotating here would duplicate
            // decoding and bypass the renderer's unchanged-handle cache.
            cache.insert(key, image::Handle::from_bytes(bytes));
        } else {
            // A malformed replacement must not leave the prior picture visible.
            cache.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests;
