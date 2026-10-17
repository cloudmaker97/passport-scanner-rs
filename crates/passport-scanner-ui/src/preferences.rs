//! Interface-only preferences. Capture preferences belong to the scanner session.
use crate::i18n::Locale;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    #[default]
    Dark,
    Light,
}

impl Appearance {
    pub const ALL: [Self; 2] = [Self::Dark, Self::Light];
    pub fn key(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
    pub fn from_key(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.key() == value)
    }
}

/// What the interface remembers between runs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Preferences {
    pub appearance: Appearance,
    /// `None` until chosen: the platform language decides.
    pub locale: Option<Locale>,
    /// Where PDFs are saved without asking; `None` asks each time. Native only.
    pub pdf_folder: Option<String>,
}

/// Where preferences live: a file beside the native settings, or browser storage.
pub trait Store: std::fmt::Debug {
    fn load(&self) -> Result<Preferences, String>;
    fn save(&self, preferences: Preferences) -> Result<(), String>;
    /// The platform's language, e.g. `de-DE`, when it can be determined.
    fn system_locale(&self) -> Option<String> {
        None
    }
}

/// Forgets everything on exit.
#[derive(Debug, Default)]
pub struct Memory(pub std::cell::RefCell<Preferences>);

impl Store for Memory {
    fn load(&self) -> Result<Preferences, String> {
        Ok(self.0.borrow().clone())
    }
    fn save(&self, preferences: Preferences) -> Result<(), String> {
        *self.0.borrow_mut() = preferences;
        Ok(())
    }
}
