//! Application preferences, separate from the vendor's device configuration.

use passport_scanner_device::CaptureProfile;
use std::{
    io,
    path::{Path, PathBuf},
};

/// Named, persisted capture choices. General remains the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaptureSelection {
    #[default]
    General,
    Identity,
    IdentityBright,
}

/// Moderate sensor exposure compensation; no per-channel image stretch.
pub const IDENTITY_BRIGHT_EXPOSURE_PERCENT: u16 = 120;

impl CaptureSelection {
    pub const fn profile(self) -> CaptureProfile {
        match self {
            Self::General => CaptureProfile::General,
            Self::Identity | Self::IdentityBright => CaptureProfile::Identity,
        }
    }

    pub const fn exposure_percent(self) -> u16 {
        match self {
            Self::IdentityBright => IDENTITY_BRIGHT_EXPOSURE_PERCENT,
            _ => 100,
        }
    }
}

pub fn profile_name(profile: CaptureSelection) -> &'static str {
    match profile {
        CaptureSelection::General => "general",
        CaptureSelection::Identity => "identity",
        CaptureSelection::IdentityBright => "identity-bright",
    }
}

pub fn parse_profile(value: &str) -> io::Result<CaptureSelection> {
    match value {
        "general" => Ok(CaptureSelection::General),
        "identity" => Ok(CaptureSelection::Identity),
        "identity-bright" => Ok(CaptureSelection::IdentityBright),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "profile must be general, identity or identity-bright",
        )),
    }
}

pub fn path() -> io::Result<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "No application settings directory is available",
            )
        })?;
    Ok(PathBuf::from(base).join("Passport Scanner").join("settings.json"))
}

/// CLI wins over environment; neither override changes the stored preference.
pub fn startup_profile(
    cli: Option<&str>,
    environment: Option<&str>,
    file: &Path,
) -> io::Result<CaptureSelection> {
    match cli.or(environment) {
        Some(value) => parse_profile(value),
        None => load_from(file),
    }
}

pub fn load_from(file: &Path) -> io::Result<CaptureSelection> {
    let bytes = match std::fs::read(file) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(CaptureSelection::General),
        Err(e) => return Err(e),
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    if value.get("version").and_then(serde_json::Value::as_u64) != Some(1) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Unsupported Passport Scanner settings version",
        ));
    }
    parse_profile(value.get("captureProfile").and_then(serde_json::Value::as_str).ok_or_else(
        || io::Error::new(io::ErrorKind::InvalidData, "Missing captureProfile setting"),
    )?)
}

/// Replace only our own small settings file after the complete write succeeds.
pub fn save_to(file: &Path, profile: CaptureSelection) -> io::Result<()> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let pending = file.with_extension("json.pending");
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "version": 1, "captureProfile": profile_name(profile)
    }))?;
    std::fs::write(&pending, bytes)?;
    std::fs::rename(&pending, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_are_strict_and_precede_storage() {
        let missing = Path::new("no-such-passport-scanner-preference-file.json");
        assert_eq!(startup_profile(None, None, missing).unwrap(), CaptureSelection::General);
        assert_eq!(
            startup_profile(None, Some("identity"), missing).unwrap(),
            CaptureSelection::Identity
        );
        assert_eq!(
            startup_profile(Some("general"), Some("identity"), missing).unwrap(),
            CaptureSelection::General
        );
        assert!(startup_profile(Some("typo"), Some("general"), missing).is_err());
        assert_eq!(parse_profile("identity-bright").unwrap(), CaptureSelection::IdentityBright);
        assert_eq!(CaptureSelection::IdentityBright.profile(), CaptureProfile::Identity);
        assert_eq!(CaptureSelection::IdentityBright.exposure_percent(), 120);
        assert_eq!(CaptureSelection::Identity.exposure_percent(), 100);
        assert_eq!(CaptureSelection::General.exposure_percent(), 100);
    }

    #[test]
    fn preferences_survive_restart_and_replace_previous_selection() {
        let dir = std::env::temp_dir().join(format!(
            "passport-scanner-preferences-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let file = dir.join("settings.json");
        save_to(&file, CaptureSelection::IdentityBright).unwrap();
        assert_eq!(load_from(&file).unwrap(), CaptureSelection::IdentityBright);
        save_to(&file, CaptureSelection::General).unwrap();
        assert_eq!(load_from(&file).unwrap(), CaptureSelection::General);
        std::fs::write(&file, b"broken").unwrap();
        assert!(load_from(&file).is_err());
        assert_eq!(
            startup_profile(Some("general"), None, &file).unwrap(),
            CaptureSelection::General
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
