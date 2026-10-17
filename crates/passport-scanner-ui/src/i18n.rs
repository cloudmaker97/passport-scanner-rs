//! Interface text. A missing translation is a compile error, not a blank label.
use passport_scanner_protocol::{FieldKind, Side, Source};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    #[default]
    De,
    En,
}

impl Locale {
    pub const ALL: [Self; 2] = [Self::De, Self::En];

    pub fn key(self) -> &'static str {
        match self {
            Self::De => "de",
            Self::En => "en",
        }
    }

    pub fn from_key(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.key() == value)
    }

    /// Map a BCP 47 tag such as `de-AT` or `en_US`; anything unknown is German.
    pub fn detect(tag: &str) -> Self {
        let primary = tag.split(['-', '_']).next().unwrap_or_default().to_ascii_lowercase();
        Self::from_key(&primary).unwrap_or_default()
    }

    /// Each language names itself, so it can be found from either one.
    pub fn native_name(self) -> &'static str {
        match self {
            Self::De => "Deutsch",
            Self::En => "English",
        }
    }

    pub fn strings(self) -> &'static Strings {
        match self {
            Self::De => &DE,
            Self::En => &EN,
        }
    }
}

#[derive(Debug)]
pub struct Strings {
    pub new_document: &'static str,
    pub scan_front: &'static str,
    pub scan_back: &'static str,
    pub pdf: &'static str,
    pub saving: &'static str,
    pub settings: &'static str,
    pub close: &'static str,
    pub document_present: &'static str,
    pub documents: &'static str,
    pub no_documents: &'static str,
    pub document: &'static str,
    pub pages: &'static str,
    pub front: &'static str,
    pub back: &'static str,
    pub side: &'static str,
    pub white_tab: &'static str,
    pub infrared_tab: &'static str,
    pub white_light: &'static str,
    pub infrared: &'static str,
    pub ultraviolet: &'static str,
    pub place_document: &'static str,
    pub side_missing: &'static str,
    pub document_data: &'static str,
    pub mrz_verified: &'static str,
    pub check_failed: &'static str,
    pub mrz_unverified: &'static str,
    pub mrz: &'static str,
    pub no_mrz: &'static str,
    pub copy_mrz: &'static str,
    pub mrz_copied: &'static str,
    pub pdf_images: &'static str,
    pub field: &'static str,
    pub given_names: &'static str,
    pub surname: &'static str,
    pub document_number: &'static str,
    pub nationality: &'static str,
    pub issuing_state: &'static str,
    pub date_of_birth: &'static str,
    pub date_of_expiry: &'static str,
    pub sex: &'static str,
    pub document_type: &'static str,
    pub optional_data: &'static str,
    pub appearance: &'static str,
    pub dark: &'static str,
    pub light: &'static str,
    pub language: &'static str,
    pub ready: &'static str,
    pub auto_scan: &'static str,
    pub buzzer: &'static str,
    pub calibration: &'static str,
    pub no_calibration: &'static str,
    pub calibration_hint: &'static str,
    pub measure: &'static str,
    pub remove_own: &'static str,
    pub server: &'static str,
    pub web_interface: &'static str,
    pub copy_address: &'static str,
    pub active: &'static str,
    pub off: &'static str,
    pub scanner_disconnected: &'static str,
    pub reconnect: &'static str,
    pub demo_mode: &'static str,
    pub connecting: &'static str,
    pub not_connected: &'static str,
    pub capturing_side: &'static str,
    pub calibrating: &'static str,
    pub calibration_missing: &'static str,
    pub stage_capturing: &'static str,
    pub stage_processing: &'static str,
    pub stage_recognising: &'static str,
    pub stage_settling: &'static str,
    pub stage_scanning: &'static str,
    pub scan_complete: &'static str,
    pub action_failed: &'static str,
    pub preview: &'static str,
    pub service_not_started: &'static str,
    pub unreadable_event: &'static str,
    pub connection_lost: &'static str,
    pub settings_not_loaded: &'static str,
    pub settings_not_saved: &'static str,
    pub pdf_saved: &'static str,
    pub pdf_downloaded: &'static str,
    pub pdf_unavailable: &'static str,
    pub save_dialog_title: &'static str,
    pub pdf_folder: &'static str,
    pub pdf_folder_ask: &'static str,
    pub pdf_name_hint: &'static str,
    pub choose_folder: &'static str,
    pub choose_folder_title: &'static str,
    pub reset: &'static str,
    pub save_as: &'static str,
    pub delete: &'static str,
    pub pdf_filter: &'static str,
    pub failure_unsupported: &'static str,
    pub failure_scanner_unavailable: &'static str,
    pub failure_profile: &'static str,
    pub failure_calibration_refused: &'static str,
    pub failure_bad_calibration: &'static str,
    pub failure_scan: &'static str,
    pub failure_sensor: &'static str,
    pub failure_bad_command: &'static str,
    pub failure_bad_setting: &'static str,
    pub failure_orientation: &'static str,
    pub failure_transport: &'static str,
    pub updates: &'static str,
    pub version: &'static str,
    pub check_updates: &'static str,
    pub checking_updates: &'static str,
    pub up_to_date: &'static str,
    pub update_available: &'static str,
    pub install_update: &'static str,
    pub installing_update: &'static str,
    pub update_installed: &'static str,
    pub restart: &'static str,
    pub update_failed: &'static str,
}

impl Strings {
    pub fn document_title(&self, number: usize) -> String {
        format!("{} {number:02}", self.document)
    }

    pub fn page_count(&self, count: usize) -> String {
        format!("{count}/2 {}", self.pages)
    }

    pub fn pdf_image_count(&self, count: usize) -> String {
        format!("PDF: {count}/4 {}", self.pdf_images)
    }

    pub fn capturing(&self, side: Side) -> String {
        format!("{} · {}", self.side_label(side), self.capturing_side)
    }

    pub fn side_label(&self, side: Side) -> &'static str {
        match side {
            Side::Front => self.front,
            Side::Back => self.back,
            _ => self.side,
        }
    }

    pub fn source_label(&self, source: Source) -> &'static str {
        match source {
            Source::White => self.white_light,
            Source::Infrared => self.infrared,
            Source::Ultraviolet => self.ultraviolet,
        }
    }

    pub fn field_label(&self, kind: FieldKind) -> &'static str {
        match kind {
            FieldKind::GivenName => self.given_names,
            FieldKind::Surname => self.surname,
            FieldKind::DocumentNumber => self.document_number,
            FieldKind::Nationality => self.nationality,
            FieldKind::IssuingState => self.issuing_state,
            FieldKind::DateOfBirth => self.date_of_birth,
            FieldKind::DateOfExpiry => self.date_of_expiry,
            FieldKind::Sex => self.sex,
            FieldKind::DocumentType => self.document_type,
            FieldKind::OptionalData => self.optional_data,
            _ => self.field,
        }
    }

    /// The service reports in English; its code says what kind of failure it was.
    pub fn failure(&self, code: &str, message: &str) -> String {
        let title = match code {
            "unsupported" => self.failure_unsupported,
            "scannerUnavailable" | "deviceNotFound" => self.failure_scanner_unavailable,
            "profileChangeFailed" => self.failure_profile,
            "calibrationRefused" => self.failure_calibration_refused,
            "badCalibration" => self.failure_bad_calibration,
            "scanFailed" => self.failure_scan,
            "sensorDisconnected" => self.failure_sensor,
            "badCommand" => self.failure_bad_command,
            "badSetting" => self.failure_bad_setting,
            "orientationFailed" => self.failure_orientation,
            "transport" => self.failure_transport,
            _ => return message.to_owned(),
        };
        if message.is_empty() { title.to_owned() } else { format!("{title}: {message}") }
    }
}

pub static DE: Strings = Strings {
    new_document: "+ Neu",
    scan_front: "Vorn scannen",
    scan_back: "Hinten scannen",
    pdf: "PDF",
    saving: "Speichern …",
    settings: "Einstellungen",
    close: "Schließen",
    document_present: "Dokument erkannt",
    documents: "Dokumente",
    no_documents: "Keine Dokumente",
    document: "Dokument",
    pages: "Seiten",
    front: "Vorderseite",
    back: "Rückseite",
    side: "Seite",
    white_tab: "Weiß",
    infrared_tab: "IR",
    white_light: "Weißlicht",
    infrared: "Infrarot",
    ultraviolet: "UV",
    place_document: "Dokument auflegen",
    side_missing: "Seite noch nicht erfasst",
    document_data: "Dokumentdaten",
    mrz_verified: "MRZ geprüft",
    check_failed: "Prüfziffer abweichend",
    mrz_unverified: "MRZ ungeprüft",
    mrz: "MRZ",
    no_mrz: "Keine MRZ",
    copy_mrz: "MRZ kopieren",
    mrz_copied: "MRZ kopiert",
    pdf_images: "Bilder",
    field: "Feld",
    given_names: "Vornamen",
    surname: "Familienname",
    document_number: "Dokumentnummer",
    nationality: "Staatsangehörigkeit",
    issuing_state: "Ausstellender Staat",
    date_of_birth: "Geburtsdatum",
    date_of_expiry: "Gültig bis",
    sex: "Geschlecht",
    document_type: "Dokumenttyp",
    optional_data: "Weitere Daten",
    appearance: "Darstellung",
    dark: "Dunkel",
    light: "Hell",
    language: "Sprache · Language",
    ready: "Bereit",
    auto_scan: "Automatisch scannen",
    buzzer: "Signalton",
    calibration: "Kalibrierung",
    no_calibration: "Keine Kalibrierung",
    calibration_hint: "Vor der Messung: Scanfläche vollständig mit weißem Papier bedecken.",
    measure: "Messen",
    remove_own: "Eigene entfernen",
    server: "Server",
    web_interface: "Weboberfläche",
    copy_address: "Adresse kopieren",
    active: "Aktiv",
    off: "Aus",
    scanner_disconnected: "Scanner nicht verbunden",
    reconnect: "Erneut verbinden",
    demo_mode: "Demo-Modus · keine Scanner-Verbindung",
    connecting: "Verbinden …",
    not_connected: "Nicht verbunden",
    capturing_side: "Aufnahme …",
    calibrating: "Kalibrieren …",
    calibration_missing: "Kalibrierung fehlt",
    stage_capturing: "Aufnahme …",
    stage_processing: "Verarbeitung …",
    stage_recognising: "MRZ lesen …",
    stage_settling: "Beleuchtung …",
    stage_scanning: "Scan läuft …",
    scan_complete: "Scan abgeschlossen",
    action_failed: "Aktion fehlgeschlagen",
    preview: "Vorschau",
    service_not_started: "Der Scanner-Dienst ist nicht gestartet.",
    unreadable_event: "Ein Ereignis des Scanner-Diensts konnte nicht gelesen werden.",
    connection_lost: "Scanner-Dienst nicht erreichbar.",
    settings_not_loaded: "Einstellungen nicht geladen",
    settings_not_saved: "Einstellungen nicht gespeichert",
    pdf_saved: "PDF gespeichert",
    pdf_downloaded: "PDF-Download gestartet",
    pdf_unavailable: "PDF-Export ist im Demo-Modus nicht verfügbar.",
    save_dialog_title: "Dokument als PDF speichern",
    pdf_folder: "PDF-Speicherort",
    pdf_folder_ask: "Bei jedem Export fragen",
    pdf_name_hint: "Dateiname: JJJJ-MM-TT-Nr-HH-MM-SS.pdf, Nr. zählt pro Tag und Ordner",
    choose_folder: "Ordner wählen …",
    choose_folder_title: "PDF-Speicherort wählen",
    reset: "Zurücksetzen",
    save_as: "Speichern unter …",
    delete: "Löschen",
    pdf_filter: "PDF-Dokument",
    failure_unsupported: "Nicht unterstützt",
    failure_scanner_unavailable: "Scanner nicht verfügbar",
    failure_profile: "Profilwechsel fehlgeschlagen",
    failure_calibration_refused: "Kalibrierung abgelehnt",
    failure_bad_calibration: "Kalibrierung ungültig",
    failure_scan: "Scan fehlgeschlagen",
    failure_sensor: "Sensor getrennt",
    failure_bad_command: "Befehl nicht verstanden",
    failure_bad_setting: "Ungültige Einstellung",
    failure_orientation: "Ausrichtung fehlgeschlagen",
    failure_transport: "Verbindungsfehler",
    updates: "Updates",
    version: "Version",
    check_updates: "Nach Updates suchen",
    checking_updates: "Suche nach Updates …",
    up_to_date: "Aktuell",
    update_available: "Update verfügbar",
    install_update: "Update installieren",
    installing_update: "Update wird installiert …",
    update_installed: "Update installiert · wirkt nach dem Neustart",
    restart: "Neu starten",
    update_failed: "Update fehlgeschlagen",
};

pub static EN: Strings = Strings {
    new_document: "+ New",
    scan_front: "Scan front",
    scan_back: "Scan back",
    pdf: "PDF",
    saving: "Saving …",
    settings: "Settings",
    close: "Close",
    document_present: "Document detected",
    documents: "Documents",
    no_documents: "No documents",
    document: "Document",
    pages: "pages",
    front: "Front",
    back: "Back",
    side: "Side",
    white_tab: "White",
    infrared_tab: "IR",
    white_light: "White light",
    infrared: "Infrared",
    ultraviolet: "UV",
    place_document: "Place a document",
    side_missing: "Side not captured yet",
    document_data: "Document data",
    mrz_verified: "MRZ verified",
    check_failed: "Check digit mismatch",
    mrz_unverified: "MRZ unverified",
    mrz: "MRZ",
    no_mrz: "No MRZ",
    copy_mrz: "Copy MRZ",
    mrz_copied: "MRZ copied",
    pdf_images: "images",
    field: "Field",
    given_names: "Given names",
    surname: "Surname",
    document_number: "Document number",
    nationality: "Nationality",
    issuing_state: "Issuing state",
    date_of_birth: "Date of birth",
    date_of_expiry: "Valid until",
    sex: "Sex",
    document_type: "Document type",
    optional_data: "Optional data",
    appearance: "Appearance",
    dark: "Dark",
    light: "Light",
    language: "Language · Sprache",
    ready: "Ready",
    auto_scan: "Scan automatically",
    buzzer: "Beep",
    calibration: "Calibration",
    no_calibration: "No calibration",
    calibration_hint: "Before measuring, cover the platen completely with white paper.",
    measure: "Measure",
    remove_own: "Remove own",
    server: "Server",
    web_interface: "Web interface",
    copy_address: "Copy address",
    active: "Active",
    off: "Off",
    scanner_disconnected: "Scanner not connected",
    reconnect: "Reconnect",
    demo_mode: "Demo mode · no scanner connection",
    connecting: "Connecting …",
    not_connected: "Not connected",
    capturing_side: "Capturing …",
    calibrating: "Calibrating …",
    calibration_missing: "Calibration missing",
    stage_capturing: "Capturing …",
    stage_processing: "Processing …",
    stage_recognising: "Reading MRZ …",
    stage_settling: "Illuminating …",
    stage_scanning: "Scanning …",
    scan_complete: "Scan complete",
    action_failed: "Action failed",
    preview: "Preview",
    service_not_started: "The scanner service is not running.",
    unreadable_event: "An event from the scanner service could not be read.",
    connection_lost: "The scanner service cannot be reached.",
    settings_not_loaded: "Settings not loaded",
    settings_not_saved: "Settings not saved",
    pdf_saved: "PDF saved",
    pdf_downloaded: "PDF download started",
    pdf_unavailable: "PDF export is not available in demo mode.",
    save_dialog_title: "Save document as PDF",
    pdf_folder: "PDF location",
    pdf_folder_ask: "Ask on every export",
    pdf_name_hint: "File name: YYYY-MM-DD-No-HH-MM-SS.pdf, No. counts per day and folder",
    choose_folder: "Choose folder …",
    choose_folder_title: "Choose PDF location",
    reset: "Reset",
    save_as: "Save as …",
    delete: "Delete",
    pdf_filter: "PDF document",
    failure_unsupported: "Not supported",
    failure_scanner_unavailable: "Scanner unavailable",
    failure_profile: "Profile change failed",
    failure_calibration_refused: "Calibration refused",
    failure_bad_calibration: "Calibration invalid",
    failure_scan: "Scan failed",
    failure_sensor: "Sensor disconnected",
    failure_bad_command: "Command not understood",
    failure_bad_setting: "Invalid setting",
    failure_orientation: "Orientation failed",
    failure_transport: "Connection error",
    updates: "Updates",
    version: "Version",
    check_updates: "Check for updates",
    checking_updates: "Checking for updates …",
    up_to_date: "Up to date",
    update_available: "Update available",
    install_update: "Install update",
    installing_update: "Installing update …",
    update_installed: "Update installed · takes effect after a restart",
    restart: "Restart",
    update_failed: "Update failed",
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Debug output lists every field, so this needs no hand-kept field list.
    fn values(strings: &Strings) -> Vec<String> {
        format!("{strings:?}")
            .split(", ")
            .filter_map(|pair| pair.split_once(": ").map(|(_, value)| value.to_owned()))
            .collect()
    }

    #[test]
    fn every_text_exists_in_every_language() {
        for locale in Locale::ALL {
            let values = values(locale.strings());
            assert!(values.len() > 100, "{locale:?}");
            assert!(values.iter().all(|value| value != "\"\""), "{locale:?} has an empty text");
        }
    }

    #[test]
    fn english_is_actually_translated() {
        let (de, en) = (values(&DE), values(&EN));
        let differing = de.iter().zip(&en).filter(|(a, b)| a != b).count();
        assert!(differing * 10 > de.len() * 8, "only {differing} of {} differ", de.len());
    }

    #[test]
    fn detects_language_tags_and_defaults_to_german() {
        assert_eq!(Locale::detect("en-US"), Locale::En);
        assert_eq!(Locale::detect("EN_gb"), Locale::En);
        assert_eq!(Locale::detect("de-AT"), Locale::De);
        assert_eq!(Locale::detect("fr-FR"), Locale::De);
        assert_eq!(Locale::detect(""), Locale::De);
    }

    #[test]
    fn service_failures_keep_detail_and_unknown_codes_pass_through() {
        assert_eq!(EN.failure("scanFailed", "Lamp stuck"), "Scan failed: Lamp stuck");
        assert_eq!(DE.failure("scanFailed", ""), "Scan fehlgeschlagen");
        assert_eq!(DE.failure("somethingNew", "Raw message"), "Raw message");
    }
}
