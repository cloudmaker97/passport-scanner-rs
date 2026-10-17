//! Hardware-free integration checks of the shared session used by all hosts.
use super::*;
use passport_scanner_protocol::{Image, Side, Source};

#[test]
fn processing_observes_a_replacement_on_the_device_thread_without_consuming_it() {
    use crate::placement::{Action, Trigger};
    use passport_scanner_core::DocumentPresence::{Absent, Full};
    let device_thread = std::thread::current().id();
    let mut trigger = Trigger::new();
    trigger.mark_scanned();
    let (release, wait) = channel();
    let readings = [Absent, Full, Full];
    let mut count = 0;
    let (result, error) = processing_with_observation(
        move || {
            assert_ne!(std::thread::current().id(), device_thread);
            wait.recv().unwrap();
            42
        },
        || {
            assert_eq!(std::thread::current().id(), device_thread);
            trigger.observe_deferred(readings[count.min(2)]);
            count += 1;
            if count == 3 {
                release.send(()).unwrap();
            }
            Ok::<_, ()>(())
        },
        std::time::Duration::from_millis(1),
    );
    assert_eq!(result.unwrap(), 42);
    assert_eq!(error, None);
    assert!(count >= 3);
    assert_eq!(trigger.observe(Full), Action::Scan);
    assert_eq!(trigger.observe(Full), Action::Wait);
}

#[test]
fn sensor_failure_preserves_completed_images_and_stops_sensor_reads() {
    let (release, wait) = channel();
    let mut reads = 0;
    let (processed, sensor_error) = processing_with_observation(
        move || {
            wait.recv().unwrap();
            scan("acquired", "document", Side::Front)
        },
        || {
            reads += 1;
            release.send(()).unwrap();
            Err("sensor disconnected")
        },
        std::time::Duration::from_millis(1),
    );
    let (listener, _events) = channel();
    let hub = Hub::with_listener(listener);
    hub.publish(Event::ScanCompleted { scan: Box::new(processed.unwrap()) });
    assert_eq!(reads, 1);
    assert_eq!(sensor_error, Some("sensor disconnected"));
    let (scans, _) = state(hub.state());
    assert_eq!(scans.len(), 1);
    assert_eq!(scans[0].images.len(), 1);
    assert_eq!(scans[0].id, "acquired");
}

fn scan(id: &str, document: &str, side: Side) -> Scan {
    Scan {
        id: id.into(),
        document: document.into(),
        side,
        at: "2026-01-01T00:00:00Z".into(),
        format: None,
        mrz: vec![],
        mrz_check: None,
        fields: vec![],
        images: vec![Image {
            source: Source::White,
            url: "data:image/jpeg;base64,SYNTHETIC".into(),
            width: 10,
            height: 6,
        }],
    }
}

fn engine() -> (Engine, Receiver<Ask>) {
    let (asks, rx) = channel();
    (Engine { hub: Arc::new(Hub::default()), asks, worker: Mutex::new(None) }, rx)
}

#[test]
fn restart_snapshot_retains_captures_and_selection_without_claiming_a_connected_device() {
    let hub = Hub::default();
    let expected = vec![scan("back", "paired", Side::Back), scan("front", "paired", Side::Front)];
    hub.restore(vec![
        Event::State { device: None, scans: expected.clone(), selected: Some("back".into()) },
        Event::AutoScanChanged { enabled: false },
    ])
    .unwrap();
    let (scans, selected) = state(hub.state());
    assert_eq!(scans, expected);
    assert_eq!(selected.as_deref(), Some("back"));
    let restored = hub.session.lock().unwrap();
    assert_eq!(restored.auto, Some(false));
    assert!(restored.resumed);
    assert!(restored.device.is_none());
}

#[test]
fn malformed_restart_snapshot_does_not_replace_the_session() {
    let hub = Hub::default();
    hub.publish(Event::ScanCompleted { scan: Box::new(scan("kept", "kept", Side::Front)) });
    assert!(
        hub.restore(vec![Event::State {
            device: None,
            scans: vec![scan("dup", "a", Side::Front), scan("dup", "b", Side::Back)],
            selected: None
        }])
        .is_err()
    );
    assert!(hub.restore(vec![Event::AutoScanChanged { enabled: true }]).is_err());
    assert_eq!(state(hub.state()).0[0].id, "kept");
}

fn state(value: Event) -> (Vec<Scan>, Option<String>) {
    if let Event::State { scans, selected, .. } = value {
        (scans, selected)
    } else {
        panic!("expected state")
    }
}

#[test]
fn retakes_replace_one_side_and_all_hosts_see_newest_first_history() {
    let (engine, _) = engine();
    for s in [
        scan("front-1", "a", Side::Front),
        scan("back-1", "a", Side::Back),
        scan("front-2", "a", Side::Front),
    ] {
        engine.hub.publish(Event::ScanCompleted { scan: Box::new(s) });
    }
    let (scans, selected) = state(engine.state());
    assert_eq!(scans.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["front-2", "back-1"]);
    assert_eq!(selected.as_deref(), Some("front-2"));
    assert_eq!(engine.scans(), scans);
    let fresh = engine.subscribe();
    assert!(matches!(
        serde_json::from_str::<Event>(&fresh.recv().unwrap()).unwrap(),
        Event::Hello { .. }
    ));
    let (reconnected, selection) = state(serde_json::from_str(&fresh.recv().unwrap()).unwrap());
    assert_eq!(reconnected, scans);
    assert_eq!(selection, selected);
}

#[test]
fn discard_by_scan_id_removes_both_sides_but_keeps_other_documents() {
    let (engine, _) = engine();
    for s in [
        scan("a-front", "a", Side::Front),
        scan("a-back", "a", Side::Back),
        scan("b-front", "b", Side::Front),
    ] {
        engine.hub.publish(Event::ScanCompleted { scan: Box::new(s) });
    }
    engine.hub.select(Some("a-back".into()));
    engine.hub.discard("a-front");
    let (scans, selected) = state(engine.state());
    assert_eq!(scans.len(), 1);
    assert_eq!(scans[0].document, "b");
    assert_eq!(selected, None);
    let result = crate::mcp::handle(serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_documents"}}), &engine.scans_without_pixels()).unwrap();
    assert_eq!(result["result"]["structuredContent"]["documents"].as_array().unwrap().len(), 1);
    assert_eq!(result["result"]["structuredContent"]["documents"][0]["documentId"], "b");
}

#[test]
fn public_commands_queue_selection_side_capture_and_new_document_in_order() {
    let (engine, rx) = engine();
    engine.send(Command::SelectScan { id: "synthetic-front".into() }).unwrap();
    engine.send(Command::SetSetting { key: "scanSide".into(), value: "back".into() }).unwrap();
    engine.send(Command::StartScan).unwrap();
    engine.send(Command::FinishDocument).unwrap();
    assert!(matches!(rx.try_recv(), Ok(Ask::Select(id)) if id == "synthetic-front"));
    assert!(matches!(rx.try_recv(), Ok(Ask::Side(Side::Back))));
    assert!(matches!(rx.try_recv(), Ok(Ask::Scan)));
    assert!(matches!(rx.try_recv(), Ok(Ask::FinishDocument)));
    engine.stop();
    assert!(engine.send(Command::StartScan).is_err());
}

#[test]
fn subscriptions_receive_current_settings_and_calibration_before_live_updates() {
    let (engine, _) = engine();
    engine.hub.publish(Event::SettingsChanged {
        settings: std::collections::BTreeMap::from([
            ("captureProfile".into(), "identity-bright".into()),
            ("profileReady".into(), "true".into()),
        ]),
    });
    engine.hub.publish(Event::AutoScanChanged { enabled: false });
    engine.hub.publish(Event::CalibrationChanged {
        source: Some("Calibration_General_ID.shd".into()),
        range: None,
    });
    let rx = engine.subscribe();
    let initial: Vec<Event> = rx.try_iter().map(|s| serde_json::from_str(&s).unwrap()).collect();
    assert!(initial.iter().any(|e| matches!(e, Event::SettingsChanged { settings } if settings.get("captureProfile").map(String::as_str) == Some("identity-bright"))));
    assert!(initial.iter().any(|e| matches!(e, Event::AutoScanChanged { enabled: false })));
    assert!(initial.iter().any(|e| matches!(e, Event::CalibrationChanged { source: Some(source), .. } if source == "Calibration_General_ID.shd")));
    engine
        .hub
        .publish(Event::ScanCompleted { scan: Box::new(scan("new", "new-doc", Side::Front)) });
    assert!(matches!(
        serde_json::from_str::<Event>(&rx.recv().unwrap()).unwrap(),
        Event::ScanCompleted { .. }
    ));
}

#[test]
fn capture_side_queues_target_and_side_in_one_message() {
    let (engine, rx) = engine();
    engine.send(Command::CaptureSide { side: Side::Back, document: Some("a".into()) }).unwrap();
    engine.send(Command::CaptureSide { side: Side::Front, document: None }).unwrap();
    assert!(
        matches!(rx.recv().unwrap(), Ask::CaptureSide { side: Side::Back, document: Some(id) } if id == "a")
    );
    assert!(matches!(rx.recv().unwrap(), Ask::CaptureSide { side: Side::Front, document: None }));
}

#[test]
fn a_mid_scan_subscriber_sees_progress_after_cached_calibration() {
    let (engine, _) = engine();
    engine.hub.publish(Event::CalibrationChanged { source: Some("factory".into()), range: None });
    engine.hub.publish(Event::ScanProgress {
        stage: passport_scanner_protocol::Stage::Recognising,
        source: None,
    });
    let events: Vec<Event> =
        engine.subscribe().try_iter().map(|s| serde_json::from_str(&s).unwrap()).collect();
    assert!(matches!(events.last(), Some(Event::ScanProgress { .. })));
    engine.hub.publish(Event::ScanCompleted { scan: Box::new(scan("a", "a", Side::Back)) });
    assert!(engine.subscribe().try_iter().all(|s| !matches!(
        serde_json::from_str::<Event>(&s).unwrap(),
        Event::ScanProgress { .. }
    )));
}

fn http(
    address: std::net::SocketAddr,
    method: &str,
    path: &str,
    headers: &str,
    body: &str,
) -> String {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(address).unwrap();
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nContent-Length: {}\r\n{headers}\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut result = String::new();
    stream.read_to_string(&mut result).unwrap();
    result
}

#[test]
fn http_and_mcp_round_trip_the_shared_session_and_stop_the_owned_listeners() {
    use crate::local_http::{Endpoint, Server};
    use std::net::{TcpListener, TcpStream};
    let (engine, rx) = engine();
    engine
        .hub
        .publish(Event::ScanCompleted { scan: Box::new(scan("capture", "document", Side::Back)) });
    let engine = Arc::new(engine);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = Server::start(listener, engine.clone(), Endpoint::Api).unwrap();
    let response = http(address, "GET", "/state", "", "");
    assert!(response.starts_with("HTTP/1.1 200"));
    let value: serde_json::Value =
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(value["selected"], "capture");
    assert_eq!(value["scans"][0]["document"], "document");
    assert!(
        http(
            address,
            "POST",
            "/command",
            "Content-Type: application/json\r\n",
            r#"{"command":"captureSide","side":"back","document":"document"}"#
        )
        .starts_with("HTTP/1.1 204")
    );
    assert!(
        matches!(rx.recv().unwrap(), Ask::CaptureSide { side: Side::Back, document: Some(id) } if id == "document")
    );
    assert!(
        http(address, "GET", "/state", "Origin: https://foreign.example\r\n", "")
            .starts_with("HTTP/1.1 403")
    );
    assert!(http(address, "GET", "/document/document/pdf", "", "").starts_with("HTTP/1.1 409"));
    assert!(http(address, "GET", "/document/pdf", "", "").starts_with("HTTP/1.1 404"));
    assert!(http(address, "GET", "/document//pdf", "", "").starts_with("HTTP/1.1 404"));
    drop(server);
    assert!(TcpStream::connect(address).is_err());

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = Server::start(listener, engine, Endpoint::Mcp).unwrap();
    let headers = "Content-Type: application/json\r\nMCP-Protocol-Version: 2025-11-25\r\n";
    assert!(
        http(
            address,
            "POST",
            "/mcp",
            headers,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
        )
        .starts_with("HTTP/1.1 202")
    );
    assert!(
        http(
            address,
            "POST",
            "/mcp",
            "Content-Type: application/json\r\n",
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#
        )
        .starts_with("HTTP/1.1 400")
    );
    let response = http(
        address,
        "POST",
        "/mcp",
        headers,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_documents"}}"#,
    );
    let value: serde_json::Value =
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(value["result"]["structuredContent"]["documents"][0]["documentId"], "document");
    assert!(http(address, "GET", "/mcp", "", "").starts_with("HTTP/1.1 405"));
    drop(server);
    assert!(TcpStream::connect(address).is_err());
}

#[test]
fn integrations_see_which_images_exist_without_their_bytes() {
    let (engine, _) = engine();
    engine.hub.publish(Event::ScanCompleted { scan: Box::new(scan("s", "d", Side::Front)) });
    let light = engine.scans_without_pixels();
    let full = engine.scans();
    assert_eq!(light[0].images.len(), full[0].images.len());
    assert_eq!(light[0].images[0].source, full[0].images[0].source);
    assert!(light[0].images[0].url.is_empty());
    assert!(!full[0].images[0].url.is_empty());
    assert_eq!(light[0].id, full[0].id);
}

#[test]
fn selecting_and_discarding_send_identifiers_not_images() {
    let (listener, events) = channel();
    let hub = Hub::with_listener(listener);
    hub.publish(Event::ScanCompleted { scan: Box::new(scan("a", "one", Side::Front)) });
    hub.publish(Event::ScanCompleted { scan: Box::new(scan("b", "two", Side::Front)) });
    let _ = events.try_iter().count();
    hub.select(Some("a".into()));
    hub.discard("b");
    let sent: Vec<Event> =
        events.try_iter().map(|json| serde_json::from_str(&json).unwrap()).collect();
    assert_eq!(
        sent,
        [
            Event::History { scans: vec!["b".into(), "a".into()], selected: Some("a".into()) },
            Event::History { scans: vec!["a".into()], selected: Some("a".into()) },
        ]
    );
}
