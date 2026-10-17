use super::*;
use std::{cell::RefCell, rc::Rc};

fn app() -> App {
    App::unconnected(Box::new(Offline), Box::<Memory>::default())
}

fn scan() -> Scan {
    Scan {
        id: "scan-a".into(),
        document: "document-a".into(),
        side: Side::Front,
        at: String::new(),
        format: None,
        mrz: vec![],
        mrz_check: None,
        fields: vec![],
        images: vec![],
    }
}

#[test]
fn new_document_state_clears_selection_without_losing_history() {
    let mut desktop = app();
    desktop.receive(Event::ScanCompleted { scan: Box::new(scan()) });
    assert!(desktop.selected_document().is_some());
    desktop.receive(Event::State { device: None, scans: vec![scan()], selected: None });
    assert!(desktop.selected_document().is_none());
    assert_eq!(desktop.documents.0.len(), 1);
}

fn pictured_scan(id: &str, document: &str, side: Side) -> Scan {
    let mut scan = scan();
    scan.id = id.into();
    scan.document = document.into();
    scan.side = side;
    scan.images = [Source::White, Source::Infrared]
        .into_iter()
        .enumerate()
        .map(|(index, source)| passport_scanner_protocol::Image {
            source,
            url: format!(
                "data:image/jpeg;base64,{}",
                base64::engine::general_purpose::STANDARD.encode([index as u8])
            ),
            width: 1,
            height: 1,
        })
        .collect();
    scan
}

#[test]
fn selecting_history_preserves_every_unchanged_renderer_handle() {
    let mut desktop = app();
    let scans: Vec<_> = (0..40)
        .map(|index| pictured_scan(&format!("s{index}"), &format!("d{index}"), Side::Front))
        .collect();
    desktop.receive(Event::State {
        device: None,
        scans: scans.clone(),
        selected: Some("s0".into()),
    });
    let ids: HashMap<_, _> =
        desktop.images.iter().map(|(key, handle)| (key.clone(), handle.id())).collect();
    for selected in ["s39", "s12", "s0"] {
        desktop.receive(Event::State {
            device: None,
            scans: scans.clone(),
            selected: Some(selected.into()),
        });
        assert_eq!(desktop.images.len(), 80);
        for (key, handle) in &desktop.images {
            assert_eq!(handle.id(), ids[key]);
        }
    }
}

#[test]
fn history_reuses_held_scans_and_their_handles_without_images_on_the_wire() {
    let mut desktop = app();
    let scans: Vec<_> = (0..3)
        .map(|index| pictured_scan(&format!("s{index}"), &format!("d{index}"), Side::Front))
        .collect();
    desktop.receive(Event::State { device: None, scans, selected: Some("s0".into()) });
    let ids: HashMap<_, _> =
        desktop.images.iter().map(|(key, handle)| (key.clone(), handle.id())).collect();

    desktop.receive(Event::History {
        scans: vec!["s0".into(), "s2".into()],
        selected: Some("s2".into()),
    });
    assert_eq!(desktop.selected.as_deref(), Some("d2"));
    assert_eq!(desktop.documents.0.len(), 2);
    assert!(desktop.documents.get("d1").is_none());
    assert_eq!(desktop.images.len(), 4);
    for (key, handle) in &desktop.images {
        assert_eq!(handle.id(), ids[key]);
    }
}

#[test]
fn history_naming_an_unknown_scan_asks_for_the_full_state() {
    let sent = Rc::default();
    let mut desktop = App::new(
        Box::new(Scripted { sent: Rc::clone(&sent), ..Default::default() }),
        Box::<Memory>::default(),
    );
    desktop.receive(Event::History { scans: vec!["missing".into()], selected: None });
    assert_eq!(sent.borrow().as_slice(), [Command::GetState]);
}

#[test]
fn changed_and_invalid_replacements_never_show_stale_images() {
    let mut desktop = app();
    let mut scan = pictured_scan("s1", "d1", Side::Front);
    desktop.receive(Event::ScanCompleted { scan: Box::new(scan.clone()) });
    let white = desktop.images["s1:0"].id();
    let infrared = desktop.images["s1:1"].id();
    scan.images[0].url = "data:image/jpeg;base64,AQID".into();
    desktop.receive(Event::State {
        device: None,
        scans: vec![scan.clone()],
        selected: Some("s1".into()),
    });
    assert_ne!(desktop.images["s1:0"].id(), white);
    assert_eq!(desktop.images["s1:1"].id(), infrared);
    scan.images[0].url = "data:image/jpeg;base64,INVALID!".into();
    desktop.receive(Event::ScanCompleted { scan: Box::new(scan.clone()) });
    assert!(!desktop.images.contains_key("s1:0"));
    assert!(
        desktop
            .image_handle(desktop.selected_document().unwrap(), Side::Front, Source::White)
            .is_none()
    );
    assert_eq!(desktop.images["s1:1"].id(), infrared);
    scan.images.clear();
    desktop.receive(Event::ScanCompleted { scan: Box::new(scan) });
    assert!(desktop.images.is_empty());
}

#[test]
fn retake_and_discard_evict_only_retired_scan_images() {
    let mut desktop = app();
    let front = pictured_scan("front", "d1", Side::Front);
    let back = pictured_scan("back", "d1", Side::Back);
    desktop.receive(Event::State {
        device: None,
        scans: vec![back.clone(), front],
        selected: Some("back".into()),
    });
    let back_handle = desktop.images["back:0"].id();
    desktop.receive(Event::ScanCompleted {
        scan: Box::new(pictured_scan("retake", "d1", Side::Front)),
    });
    assert_eq!(desktop.images.len(), 4);
    assert!(!desktop.images.contains_key("front:0"));
    assert_eq!(desktop.images["back:0"].id(), back_handle);
    desktop.receive(Event::State { device: None, scans: vec![back], selected: None });
    assert_eq!(desktop.images.len(), 2);
    assert_eq!(desktop.images["back:0"].id(), back_handle);
    desktop.receive(Event::State { device: None, scans: vec![], selected: None });
    assert!(desktop.images.is_empty());
    assert!(desktop.selected_document().is_none());
}

#[test]
fn capture_atomically_carries_side_and_selected_document_or_new_pair() {
    let mut desktop = app();
    assert_eq!(
        desktop.capture_command(Side::Front),
        Command::CaptureSide { side: Side::Front, document: None }
    );
    desktop.receive(Event::ScanCompleted { scan: Box::new(scan()) });
    assert_eq!(
        desktop.capture_command(Side::Back),
        Command::CaptureSide { side: Side::Back, document: Some("document-a".into()) }
    );
    desktop.receive(Event::State { device: None, scans: vec![scan()], selected: None });
    assert_eq!(
        desktop.capture_command(Side::Front),
        Command::CaptureSide { side: Side::Front, document: None }
    );
}

#[test]
fn recognition_progress_keeps_controls_busy_until_completion() {
    let mut desktop = app();
    desktop.receive(Event::ScanProgress { stage: Stage::Recognising, source: None });
    assert!(desktop.busy);
    assert_eq!(desktop.status.label(&i18n::DE), "MRZ lesen …");
    desktop.receive(Event::ScanCompleted { scan: Box::new(scan()) });
    assert!(!desktop.busy);
}

#[test]
fn calibration_completion_clears_progress_label() {
    let mut desktop = app();
    desktop.profile_ready = true;
    desktop.busy = true;
    desktop.status = Status::Calibrating;
    desktop
        .receive(Event::CalibrationChanged { source: Some("reference.shd".into()), range: None });
    assert!(!desktop.busy);
    assert_eq!(desktop.status, Status::Ready);
}

#[test]
fn settings_and_preview_switches_preserve_document_and_capture_target() {
    let mut desktop = app();
    desktop.receive(Event::ScanCompleted { scan: Box::new(scan()) });
    assert_eq!(desktop.appearance, Appearance::Dark);
    assert!(desktop.theme().palette().background.r < 0.1);
    let expected = desktop.capture_command(Side::Back);
    let _ = desktop.update(Message::OpenSettings);
    assert!(desktop.settings_open);
    let _ = desktop.update(Message::CloseSettings);
    let _ = desktop.update(Message::PreviewSide(Side::Back));
    let _ = desktop.update(Message::PreviewSource(Source::Infrared));
    assert!(!desktop.settings_open);
    assert_eq!(desktop.preview_side, Side::Back);
    assert_eq!(desktop.preview_source, Source::Infrared);
    assert_eq!(desktop.capture_command(Side::Back), expected);
    assert_eq!(desktop.documents.0.len(), 1);
    desktop.appearance = Appearance::Light;
    assert!(desktop.theme().palette().background.r > 0.9);
}

#[test]
fn new_capture_selects_its_side_and_refresh_preserves_preview_choice() {
    let mut desktop = app();
    let mut back = scan();
    back.side = Side::Back;
    desktop.receive(Event::ScanCompleted { scan: Box::new(back.clone()) });
    assert_eq!(desktop.preview_side, Side::Back);
    let _ = desktop.update(Message::PreviewSource(Source::Infrared));
    desktop.receive(Event::State {
        device: None,
        scans: vec![back.clone()],
        selected: Some(back.id),
    });
    assert_eq!(desktop.preview_side, Side::Back);
    assert_eq!(desktop.preview_source, Source::Infrared);
}

#[test]
fn offline_backend_never_enables_scanning_even_on_reconnect() {
    let mut desktop = app();
    desktop.receive(Event::State {
        device: None,
        scans: vec![scan()],
        selected: Some("scan-a".into()),
    });
    let _ = desktop.update(Message::Reconnect);
    assert!(!desktop.backend.connected());
    assert!(!desktop.can_scan());
    assert_eq!(desktop.status, Status::NotConnected);
    assert_eq!(desktop.selected.as_deref(), Some("document-a"));
}

/// Shares its memory with the test, so persistence is observable after a move.
#[derive(Debug, Default, Clone)]
struct Shared(Rc<RefCell<Option<Preferences>>>, Option<&'static str>);

impl Store for Shared {
    fn load(&self) -> Result<Preferences, String> {
        Ok(self.0.borrow().clone().unwrap_or_default())
    }
    fn save(&self, preferences: Preferences) -> Result<(), String> {
        *self.0.borrow_mut() = Some(preferences);
        Ok(())
    }
    fn system_locale(&self) -> Option<String> {
        self.1.map(String::from)
    }
}

#[test]
fn language_follows_platform_until_chosen_then_persists_and_relabels() {
    let store = Shared(Rc::default(), Some("en-US"));
    let mut english = App::new(Box::new(Offline), Box::new(store.clone()));
    assert_eq!(english.locale, Locale::En);
    assert_eq!(english.status.label(english.t()), "Not connected");
    let _ = english.update(Message::Locale(Locale::De));
    assert_eq!(english.status.label(english.t()), "Nicht verbunden");
    assert_eq!(store.0.borrow().clone().unwrap().locale, Some(Locale::De));
    // A saved choice beats the platform language on the next start.
    let reopened = App::new(Box::new(Offline), Box::new(store));
    assert_eq!(reopened.locale, Locale::De);
    assert_eq!(App::new(Box::new(Offline), Box::new(Shared::default())).locale, Locale::De);
}

#[test]
fn appearance_change_keeps_the_chosen_language() {
    let store = Shared(Rc::default(), Some("en"));
    let mut desktop = App::new(Box::new(Offline), Box::new(store.clone()));
    let _ = desktop.update(Message::Appearance(Appearance::Light));
    assert_eq!(
        *store.0.borrow(),
        Some(Preferences {
            appearance: Appearance::Light,
            locale: Some(Locale::En),
            pdf_folder: None
        })
    );
}

/// Feeds scripted inbound items; records what the interface sends.
#[derive(Debug, Default)]
struct Scripted {
    inbound: Vec<Inbound>,
    sent: Rc<RefCell<Vec<Command>>>,
    open: bool,
    exported_to: Rc<RefCell<Option<Option<String>>>>,
    requests: Rc<RefCell<Vec<ExportRequest>>>,
    autosave: bool,
    updates: bool,
    installed: Rc<RefCell<Vec<Update>>>,
}

impl Backend for Scripted {
    fn connect(&mut self) -> Result<Vec<Event>, String> {
        self.open = true;
        Ok(vec![])
    }
    fn connected(&self) -> bool {
        self.open
    }
    fn send(&mut self, command: Command) -> Result<(), String> {
        self.sent.borrow_mut().push(command);
        Ok(())
    }
    fn poll(&mut self) -> Vec<Inbound> {
        std::mem::take(&mut self.inbound)
    }
    fn export_pdf(&self, _: String, request: ExportRequest) -> Task<Message> {
        *self.exported_to.borrow_mut() = Some(request.folder.clone());
        self.requests.borrow_mut().push(request);
        Task::none()
    }
    fn picks_folders(&self) -> bool {
        true
    }
    fn saves_automatically(&self) -> bool {
        self.autosave
    }
    fn updates(&self) -> bool {
        self.updates
    }
    fn install_update(&self, update: Update) -> Task<Message> {
        self.installed.borrow_mut().push(update);
        Task::none()
    }
}

#[test]
fn updates_are_looked_for_once_installed_on_request_and_then_restart() {
    let installed = Rc::new(RefCell::new(Vec::new()));
    let mut app = App::new(
        Box::new(Scripted { updates: true, installed: installed.clone(), ..Default::default() }),
        Box::<Memory>::default(),
    );
    let _ = app.update(Message::Tick);
    assert_eq!(app.updating, Updating::Checking);
    let update = Update { version: "0.2.0".into(), download: "archive".into(), digest: None };
    let _ = app.update(Message::UpdateChecked(Ok(Some(update.clone()))));
    // Only the first tick looks; later ticks leave the offer alone.
    let _ = app.update(Message::Tick);
    assert_eq!(app.updating, Updating::Available(update.clone()));
    let _ = app.update(Message::Restart);
    assert_eq!(app.updating, Updating::Available(update.clone()));
    let _ = app.update(Message::InstallUpdate);
    assert_eq!(*installed.borrow(), vec![update.clone()]);
    let _ = app.update(Message::InstallUpdate);
    assert_eq!(installed.borrow().len(), 1, "installing twice at once");
    let _ = app.update(Message::UpdateInstalled(Err("disk full".into())));
    assert!(matches!(&app.updating, Updating::Failed(error) if error.ends_with("disk full")));
    let _ = app.update(Message::UpdateChecked(Ok(Some(update.clone()))));
    let _ = app.update(Message::InstallUpdate);
    let _ = app.update(Message::UpdateInstalled(Ok(())));
    assert_eq!(app.updating, Updating::Installed("0.2.0".into()));
    // Nothing to look for while an installed update waits for its restart.
    let _ = app.update(Message::CheckUpdate);
    assert_eq!(app.updating, Updating::Installed("0.2.0".into()));
}

#[test]
fn a_backend_without_updates_never_looks() {
    let mut app = App::new(Box::new(Scripted::default()), Box::<Memory>::default());
    let _ = app.update(Message::Tick);
    let _ = app.update(Message::CheckUpdate);
    assert_eq!(app.updating, Updating::Idle);
}

#[test]
fn lost_connection_is_reported_and_cleared_when_it_returns() {
    let mut desktop = App::new(
        Box::new(Scripted { inbound: vec![Inbound::Connection(false)], ..Default::default() }),
        Box::<Memory>::default(),
    );
    desktop.busy = true;
    let _ = desktop.update(Message::Tick);
    assert!(!desktop.busy);
    assert_eq!(desktop.status, Status::NotConnected);
    assert_eq!(desktop.notice.as_ref().unwrap().0, i18n::DE.connection_lost);
    desktop.backend = Box::new(Scripted {
        inbound: vec![Inbound::Connection(true)],
        open: true,
        ..Default::default()
    });
    let _ = desktop.update(Message::Tick);
    assert!(desktop.notice.is_none());
}

#[test]
fn unreadable_events_are_reported_not_ignored() {
    let mut desktop = App::new(
        Box::new(Scripted { inbound: vec![Inbound::Unreadable], ..Default::default() }),
        Box::<Memory>::default(),
    );
    let _ = desktop.update(Message::Tick);
    assert_eq!(desktop.notice.as_ref().unwrap().0, i18n::DE.unreadable_event);
}

#[test]
fn connecting_relies_on_the_replayed_state_and_export_outcomes_are_explained() {
    let sent = Rc::default();
    let mut desktop = App::new(
        Box::new(Scripted { sent: Rc::clone(&sent), ..Default::default() }),
        Box::<Memory>::default(),
    );
    // The session replays its state on subscribing; asking again would send
    // every image to every other client as well.
    assert!(sent.borrow().is_empty());
    desktop.exporting = true;
    let _ = desktop.update(Message::Exported(Ok(ExportOutcome::Cancelled)));
    assert!(!desktop.exporting && desktop.notice.is_none());
    let _ = desktop.update(Message::Exported(Ok(ExportOutcome::Downloaded)));
    assert_eq!(desktop.notice, Some((i18n::DE.pdf_downloaded.into(), false)));
    let _ = desktop.update(Message::Exported(Ok(ExportOutcome::Saved("C:/a.pdf".into()))));
    assert_eq!(desktop.notice, Some(("PDF gespeichert: C:/a.pdf".into(), false)));
    let _ = desktop.update(Message::Exported(Ok(ExportOutcome::Unavailable)));
    assert!(desktop.notice.as_ref().unwrap().1);
}

#[test]
fn demo_backend_scans_a_complete_verified_document_through_the_interface() {
    let mut desktop = App::new(Box::new(mock::Mock::new()), Box::<Memory>::default());
    for _ in 0..10 {
        let _ = desktop.update(Message::Tick);
    }
    assert!(desktop.can_scan(), "{:?}", desktop.status);
    for side in [Side::Front, Side::Back] {
        let _ = desktop.update(Message::Capture(side));
        assert!(desktop.busy);
        for _ in 0..10 {
            let _ = desktop.update(Message::Tick);
        }
        assert_eq!(desktop.status, Status::ScanComplete);
    }
    let document = desktop.selected_document().unwrap();
    assert_eq!(document.image_count(), 4);
    assert_eq!(model::verification(document.recognised().unwrap()), model::Verification::Verified);
    assert_eq!(desktop.images.len(), 4);
    let _ = desktop.update(Message::CopyMrz);
    assert_eq!(desktop.notice, Some((i18n::DE.mrz_copied.into(), false)));
}

#[test]
fn chosen_pdf_folder_persists_and_reaches_the_export_until_reset() {
    let store = Shared(Rc::default(), None);
    let exported_to = Rc::default();
    let mut desktop = App::new(
        Box::new(Scripted { exported_to: Rc::clone(&exported_to), ..Default::default() }),
        Box::new(store.clone()),
    );
    let _ = desktop.update(Message::FolderChosen(Some("D:/Scans".into())));
    assert_eq!(store.0.borrow().clone().unwrap().pdf_folder.as_deref(), Some("D:/Scans"));
    // A cancelled dialog keeps the folder.
    let _ = desktop.update(Message::FolderChosen(None));
    assert_eq!(desktop.pdf_folder.as_deref(), Some("D:/Scans"));
    for side in [Side::Front, Side::Back] {
        let mut scan = pictured_scan(&format!("{side:?}"), "d1", side);
        scan.images[0].url = "data:image/jpeg;base64,AA==".into();
        desktop.receive(Event::ScanCompleted { scan: Box::new(scan) });
    }
    let _ = desktop.update(Message::Export);
    assert_eq!(*exported_to.borrow(), Some(Some("D:/Scans".into())));
    let reopened = App::new(Box::new(Offline), Box::new(store.clone()));
    assert_eq!(reopened.pdf_folder.as_deref(), Some("D:/Scans"));
    let _ = desktop.update(Message::ResetFolder);
    assert_eq!(store.0.borrow().clone().unwrap().pdf_folder, None);
    desktop.exporting = false;
    let _ = desktop.update(Message::Export);
    assert_eq!(*exported_to.borrow(), Some(None));
}

#[test]
fn history_menu_saves_as_with_a_dialog_and_deletes_the_whole_document() {
    let exported_to = Rc::default();
    let sent = Rc::default();
    let mut desktop = App::new(
        Box::new(Scripted {
            exported_to: Rc::clone(&exported_to),
            sent: Rc::clone(&sent),
            ..Default::default()
        }),
        Box::<Memory>::default(),
    );
    desktop.pdf_folder = Some("D:/Scans".into());
    for side in [Side::Front, Side::Back] {
        let scan = pictured_scan(&format!("{side:?}"), "d1", side);
        desktop.receive(Event::ScanCompleted { scan: Box::new(scan) });
    }
    let _ = desktop.update(Message::HistoryMenu(Some("d1".into())));
    assert_eq!(desktop.history_menu.as_deref(), Some("d1"));
    let _ = desktop.update(Message::CloseSettings);
    assert_eq!(desktop.history_menu, None, "Escape closes the menu");
    let _ = desktop.update(Message::HistoryMenu(Some("d1".into())));
    let _ = desktop.update(Message::SaveDocumentAs("d1".into()));
    assert_eq!(*exported_to.borrow(), Some(None), "save as always asks, ignoring the folder");
    assert!(desktop.history_menu.is_none());
    desktop.exporting = false;
    let _ = desktop.update(Message::DeleteDocument("d1".into()));
    assert!(matches!(sent.borrow().last(), Some(Command::Discard { .. })));
    // An incomplete document cannot be exported, from the menu or otherwise.
    *exported_to.borrow_mut() = None;
    desktop.receive(Event::State { device: None, scans: vec![scan()], selected: None });
    let _ = desktop.update(Message::SaveDocumentAs("document-a".into()));
    assert_eq!(*exported_to.borrow(), None);
}

#[test]
fn demo_delete_removes_both_sides() {
    let mut desktop = App::new(Box::new(mock::Mock::new()), Box::<Memory>::default());
    for side in [Side::Front, Side::Back] {
        let _ = desktop.update(Message::Capture(side));
        for _ in 0..10 {
            let _ = desktop.update(Message::Tick);
        }
    }
    let id = desktop.documents.0[0].id.clone();
    let _ = desktop.update(Message::DeleteDocument(id));
    for _ in 0..5 {
        let _ = desktop.update(Message::Tick);
    }
    assert!(desktop.documents.0.is_empty());
    assert!(desktop.images.is_empty());
}

#[test]
fn both_sides_save_by_themselves_and_a_rescan_replaces_that_file() {
    let requests: Rc<RefCell<Vec<ExportRequest>>> = Rc::default();
    let mut desktop = App::new(
        Box::new(Scripted { requests: Rc::clone(&requests), autosave: true, ..Default::default() }),
        Box::<Memory>::default(),
    );
    desktop.pdf_folder = Some("D:/Scans".into());
    let completed = |side: Side| Event::ScanCompleted {
        scan: Box::new(pictured_scan(&format!("{side:?}"), "d1", side)),
    };
    desktop.receive(completed(Side::Front));
    let _ = desktop.update(Message::Tick);
    assert!(requests.borrow().is_empty(), "one side is not a document yet");
    desktop.receive(completed(Side::Back));
    let _ = desktop.update(Message::Tick);
    {
        let requests = requests.borrow();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].folder.as_deref(), Some("D:/Scans"));
        assert!(!requests[0].ask && requests[0].path.is_none());
    }
    // A rescan finishing while the first save runs waits for it.
    desktop.receive(completed(Side::Front));
    let _ = desktop.update(Message::Tick);
    assert_eq!(requests.borrow().len(), 1);
    let _ = desktop.update(Message::Exported(Ok(ExportOutcome::Saved("D:/Scans/a.pdf".into()))));
    assert_eq!(requests.borrow().len(), 2);
    assert_eq!(requests.borrow()[1].path.as_deref(), Some("D:/Scans/a.pdf"));
    let _ = desktop.update(Message::Exported(Ok(ExportOutcome::Saved("D:/Scans/a.pdf".into()))));
    // A manual export still makes a new file.
    let _ = desktop.update(Message::Export);
    assert!(requests.borrow()[2].ask && requests.borrow()[2].path.is_none());
}
