#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
//! Native window around the shared interface. Owns the scanner session in-process.

mod store;

use iced::Task;
use passport_scanner_app::desktop_host::Host;
use passport_scanner_protocol::{Command, Event};
use passport_scanner_ui::{
    App, Backend, ExportOutcome, ExportRequest, Inbound, LocalServices, Message, Offline,
    mock::Mock,
    pdf_name::{self, Stamp},
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, mpsc::Receiver},
};

fn main() -> iced::Result {
    let icon_pixels = ::image::load_from_memory(include_bytes!("../../../icon.png"))
        .expect("the embedded application icon is a valid PNG")
        .into_rgba8();
    let icon = iced::window::icon::from_rgba(
        icon_pixels.as_raw().clone(),
        icon_pixels.width(),
        icon_pixels.height(),
    )
    .expect("the decoded application icon has valid RGBA dimensions");
    iced::application(|| App::new(backend(), Box::new(store::File)), App::update, App::view)
        .title(passport_scanner_ui::TITLE)
        .theme(App::theme)
        .settings(passport_scanner_ui::settings())
        .subscription(App::subscription)
        .window(iced::window::Settings {
            size: iced::Size::new(1440.0, 960.0),
            min_size: Some(iced::Size::new(1120.0, 760.0)),
            icon: Some(icon),
            ..Default::default()
        })
        .centered()
        .run()
}

/// `PASSPORT_SCANNER_UI_MOCK=1` simulates a scanner; a render session never opens one.
fn backend() -> Box<dyn Backend> {
    if cfg!(feature = "render-smoke")
        && std::env::var_os("PASSPORT_SCANNER_UI_SMOKE_SESSION").is_some()
    {
        return Box::new(Offline);
    }
    if std::env::var("PASSPORT_SCANNER_UI_MOCK").as_deref() == Ok("1") {
        return Box::new(Mock::new());
    }
    Box::new(HostBackend::default())
}

/// The scanner session, running in this process.
#[derive(Debug, Default)]
struct HostBackend {
    host: Option<Session>,
    events: Option<Receiver<String>>,
}

/// The host, shared with export tasks; also owns the optional local services.
#[derive(Debug)]
struct Session(Arc<Host>);

impl LocalServices for Session {
    fn web_running(&self) -> bool {
        self.0.web_running()
    }
    fn set_web_enabled(&self, enabled: bool) -> Result<(), String> {
        self.0.set_web_enabled(enabled).map_err(|e| e.to_string())
    }
    fn mcp_running(&self) -> bool {
        self.0.mcp_running()
    }
    fn set_mcp_enabled(&self, enabled: bool) -> Result<(), String> {
        self.0.set_mcp_enabled(enabled).map_err(|e| e.to_string())
    }
}

impl Backend for HostBackend {
    fn connect(&mut self) -> Result<Vec<Event>, String> {
        if self.host.is_some() {
            return Ok(Vec::new());
        }
        let host = Host::start().map_err(|e| e.to_string())?;
        self.events = Some(host.subscribe());
        let state = host.state();
        self.host = Some(Session(Arc::new(host)));
        Ok(vec![state])
    }

    fn connected(&self) -> bool {
        self.host.is_some()
    }

    fn send(&mut self, command: Command) -> Result<(), String> {
        let host = self.host.as_ref().ok_or_else(String::new)?;
        host.0.send(command).map_err(|e| e.to_string())
    }

    fn poll(&mut self) -> Vec<Inbound> {
        let Some(events) = &self.events else { return Vec::new() };
        events
            .try_iter()
            .take(64)
            .map(|raw| serde_json::from_str(&raw).map_or(Inbound::Unreadable, Inbound::Event))
            .collect()
    }

    fn export_pdf(&self, document_id: String, request: ExportRequest) -> Task<Message> {
        let Some(host) = self.host.as_ref().map(|session| Arc::clone(&session.0)) else {
            return Task::done(Message::Exported(Ok(ExportOutcome::Unavailable)));
        };
        let stamp = now();
        Task::perform(
            async move {
                let path = match request.folder {
                    Some(folder) => {
                        let folder = PathBuf::from(folder);
                        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
                        unused_name(&folder, stamp)
                    }
                    None => {
                        let start = dirs::document_dir().unwrap_or_default();
                        let suggested = unused_name(&start, stamp);
                        let name = suggested.file_name().unwrap_or_default().to_string_lossy();
                        let Some(file) = rfd::AsyncFileDialog::new()
                            .set_title(&request.title)
                            .set_directory(&start)
                            .set_file_name(name)
                            .add_filter(&request.filter, &["pdf"])
                            .save_file()
                            .await
                        else {
                            return Ok(ExportOutcome::Cancelled);
                        };
                        let mut path = file.path().to_path_buf();
                        if path.extension().is_none() {
                            path.set_extension("pdf");
                        }
                        path
                    }
                };
                host.export_pdf(&document_id, &path).map_err(|e| e.to_string())?;
                Ok(ExportOutcome::Saved(path.display().to_string()))
            },
            Message::Exported,
        )
    }

    fn picks_folders(&self) -> bool {
        true
    }

    fn choose_folder(&self, title: String) -> Task<Message> {
        Task::perform(
            async move {
                rfd::AsyncFileDialog::new()
                    .set_title(&title)
                    .pick_folder()
                    .await
                    .map(|folder| folder.path().display().to_string())
            },
            Message::FolderChosen,
        )
    }

    fn services(&self) -> Option<&dyn LocalServices> {
        self.host.as_ref().map(|session| session as &dyn LocalServices)
    }

    fn saved_profile(&self) -> Option<String> {
        let path = passport_scanner_app::preferences::path().ok()?;
        let saved = passport_scanner_app::preferences::load_from(&path).ok()?;
        Some(passport_scanner_app::preferences::profile_name(saved).to_owned())
    }
}

/// Local wall-clock time; UTC if the platform cannot say what the offset is.
fn now() -> Stamp {
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    Stamp {
        year: now.year(),
        month: now.month().into(),
        day: now.day(),
        hour: now.hour(),
        minute: now.minute(),
        second: now.second(),
    }
}

/// The next export name in `folder`; never an existing file.
fn unused_name(folder: &Path, stamp: Stamp) -> PathBuf {
    let existing: Vec<String> = std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    let mut number = pdf_name::next_number(&stamp.date(), existing.iter().map(String::as_str));
    loop {
        let path = folder.join(stamp.file_name(number));
        if !path.exists() {
            return path;
        }
        number += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_names_count_per_day_in_the_folder_and_never_overwrite() {
        let folder = std::env::temp_dir().join(format!(
            "passport-scanner-pdf-names-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&folder).unwrap();
        let stamp = Stamp { year: 2026, month: 10, day: 17, hour: 14, minute: 3, second: 9 };
        let first = unused_name(&folder, stamp);
        assert_eq!(first.file_name().unwrap(), "2026-10-17--001--14-03-09.pdf");
        std::fs::write(&first, b"%PDF").unwrap();
        std::fs::write(folder.join("2026-10-17--004--10-00-00.pdf"), b"%PDF").unwrap();
        std::fs::write(folder.join("2026-10-16--009--10-00-00.pdf"), b"%PDF").unwrap();
        assert_eq!(
            unused_name(&folder, stamp).file_name().unwrap(),
            "2026-10-17--005--14-03-09.pdf"
        );
        let next_day = Stamp { day: 18, ..stamp };
        assert_eq!(
            unused_name(&folder, next_day).file_name().unwrap(),
            "2026-10-18--001--14-03-09.pdf"
        );
        assert_eq!(
            unused_name(&folder.join("missing"), stamp).file_name().unwrap(),
            "2026-10-17--001--14-03-09.pdf"
        );
        std::fs::remove_dir_all(folder).unwrap();
    }
}
