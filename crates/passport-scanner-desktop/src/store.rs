//! Interface preferences on disk. Capture preferences belong to the scanner host.
use passport_scanner_ui::{Appearance, Locale, Preferences, Store};
use std::{
    io,
    path::{Path, PathBuf},
};

/// `ui-settings.json` beside the host's `settings.json`.
#[derive(Debug)]
pub(crate) struct File;

impl Store for File {
    fn load(&self) -> Result<Preferences, String> {
        path().and_then(|path| load_from(&path)).map_err(|e| e.to_string())
    }
    fn save(&self, preferences: Preferences) -> Result<(), String> {
        path().and_then(|path| save_to(&path, preferences)).map_err(|e| e.to_string())
    }
    fn system_locale(&self) -> Option<String> {
        sys_locale::get_locale()
    }
}

fn path() -> io::Result<PathBuf> {
    Ok(passport_scanner_app::preferences::path()?.with_file_name("ui-settings.json"))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn load_from(path: &Path) -> io::Result<Preferences> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Preferences::default()),
        Err(error) => return Err(error),
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    if value.get("version").and_then(|v| v.as_u64()) != Some(1) {
        return Err(invalid("unknown ui-settings version"));
    }
    let appearance = value
        .get("appearance")
        .and_then(|v| v.as_str())
        .and_then(Appearance::from_key)
        .ok_or_else(|| invalid("unknown appearance"))?;
    // Absent in files written before languages existed: follow the platform.
    let locale = match value.get("locale").and_then(|v| v.as_str()) {
        None => None,
        Some(key) => Some(Locale::from_key(key).ok_or_else(|| invalid("unknown locale"))?),
    };
    let pdf_folder = match value.get("pdfFolder") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(folder)) if !folder.is_empty() => Some(folder.clone()),
        Some(_) => return Err(invalid("invalid pdfFolder")),
    };
    Ok(Preferences { appearance, locale, pdf_folder })
}

fn save_to(path: &Path, preferences: Preferences) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let mut value = serde_json::json!({"version": 1, "appearance": preferences.appearance.key()});
    if let Some(locale) = preferences.locale {
        value["locale"] = locale.key().into();
    }
    if let Some(folder) = &preferences.pdf_folder {
        value["pdfFolder"] = folder.as_str().into();
    }
    let pending = path.with_extension("json.pending");
    std::fs::write(&pending, serde_json::to_vec_pretty(&value)?)?;
    std::fs::rename(pending, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_roundtrip_without_touching_capture_preferences() {
        let folder = std::env::temp_dir().join(format!(
            "passport-scanner-ui-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("ui-settings.json");
        let capture = folder.join("settings.json");
        std::fs::write(&capture, "capture-preferences-untouched").unwrap();
        assert_eq!(load_from(&path).unwrap(), Preferences::default());
        let light_english = Preferences {
            appearance: Appearance::Light,
            locale: Some(Locale::En),
            pdf_folder: Some(r"D:\Scans\Ausweise".into()),
        };
        save_to(&path, light_english.clone()).unwrap();
        assert_eq!(load_from(&path).unwrap(), light_english);
        save_to(&path, Preferences::default()).unwrap();
        assert_eq!(load_from(&path).unwrap(), Preferences::default());
        assert_eq!(std::fs::read_to_string(&capture).unwrap(), "capture-preferences-untouched");
        // Files from before languages existed still load.
        std::fs::write(&path, r#"{"version":1,"appearance":"light"}"#).unwrap();
        assert_eq!(
            load_from(&path).unwrap(),
            Preferences { appearance: Appearance::Light, locale: None, pdf_folder: None }
        );
        std::fs::write(&path, r#"{"version":1,"appearance":"unknown"}"#).unwrap();
        assert!(load_from(&path).is_err());
        std::fs::write(&path, r#"{"version":1,"appearance":"dark","locale":"xx"}"#).unwrap();
        assert!(load_from(&path).is_err());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(capture).unwrap();
        std::fs::remove_dir(folder).unwrap();
    }
}
