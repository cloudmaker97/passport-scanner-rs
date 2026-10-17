//! What the interface needs from whoever carries its messages.
//!
//! The native client runs the scanner session in-process; the browser client
//! reaches the same session over the local HTTP service. Both speak only the
//! wire contract, so the interface cannot tell them apart except through the
//! optional capabilities below.
use crate::Message;
use iced::Task;
use passport_scanner_protocol::{Command, Event};

/// Something arriving from the session.
#[derive(Debug, Clone, PartialEq)]
pub enum Inbound {
    Event(Event),
    /// A message arrived that does not parse as an [`Event`].
    Unreadable,
    /// The link to the session went down (`false`) or came back (`true`).
    Connection(bool),
}

/// What a PDF export needs besides the document. The file is named by
/// [`crate::pdf_name`]; dialog texts are already in the interface language.
#[derive(Debug, Clone)]
pub struct ExportRequest {
    pub title: String,
    pub filter: String,
    /// Save here without asking; `None` asks where to save.
    pub folder: Option<String>,
}

/// How a PDF export ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportOutcome {
    /// Written to this path.
    Saved(String),
    /// Handed to the browser as a download.
    Downloaded,
    /// The person closed the save dialog.
    Cancelled,
    /// This backend cannot export.
    Unavailable,
}

pub trait Backend: std::fmt::Debug {
    /// Open the session; idempotent. Returns events describing the current state.
    fn connect(&mut self) -> Result<Vec<Event>, String>;
    fn connected(&self) -> bool;
    fn send(&mut self, command: Command) -> Result<(), String>;
    /// Everything that arrived since the last call.
    fn poll(&mut self) -> Vec<Inbound>;
    /// Must finish with [`Message::Exported`].
    fn export_pdf(&self, document_id: String, request: ExportRequest) -> Task<Message>;
    fn copy_text(&self, text: String) -> Task<Message> {
        iced::clipboard::write(text)
    }
    /// Whether a permanent export folder can be chosen; a browser cannot.
    fn picks_folders(&self) -> bool {
        false
    }
    /// Ask for that folder; finishes with [`Message::FolderChosen`].
    fn choose_folder(&self, _title: String) -> Task<Message> {
        Task::none()
    }
    /// Local web and MCP services, which only the native session owns.
    fn services(&self) -> Option<&dyn LocalServices> {
        None
    }
    /// The capture profile the session will start with, if known before it reports.
    fn saved_profile(&self) -> Option<String> {
        None
    }
    /// A backend that only simulates a scanner says so in the interface.
    fn is_demo(&self) -> bool {
        false
    }
}

pub trait LocalServices {
    fn web_running(&self) -> bool;
    fn set_web_enabled(&self, enabled: bool) -> Result<(), String>;
    fn mcp_running(&self) -> bool;
    fn set_mcp_enabled(&self, enabled: bool) -> Result<(), String>;
}

/// Never connects. Used where showing a saved session must not open a scanner.
#[derive(Debug, Default)]
pub struct Offline;

impl Backend for Offline {
    fn connect(&mut self) -> Result<Vec<Event>, String> {
        Err(String::new())
    }
    fn connected(&self) -> bool {
        false
    }
    fn send(&mut self, _: Command) -> Result<(), String> {
        Err(String::new())
    }
    fn poll(&mut self) -> Vec<Inbound> {
        Vec::new()
    }
    fn export_pdf(&self, _: String, _: ExportRequest) -> Task<Message> {
        Task::done(Message::Exported(Ok(ExportOutcome::Unavailable)))
    }
}
