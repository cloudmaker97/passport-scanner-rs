//! Live ID-side scan/session verification. Writes private evidence only to the requested folder.
//! Run with the same ID side left on the platen: verify_session OUTPUT COUNT front|back.
use base64::Engine as _;
use passport_scanner_app::{desktop_host::Host, local_http};
use passport_scanner_protocol::{Command, Event};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().ok_or("Usage: verify_session OUTPUT COUNT front|back")?);
    let count: usize = args.next().unwrap_or_else(|| "1".into()).parse()?;
    let side = match args.next().as_deref() {
        Some("front") => passport_scanner_protocol::Side::Front,
        Some("back") | None => passport_scanner_protocol::Side::Back,
        _ => return Err("Side must be front or back".into()),
    };
    if !(1..=100).contains(&count) {
        return Err("COUNT must be 1..100".into());
    }
    std::fs::create_dir_all(&out)?;
    let host = Host::start()?;
    let events = host.subscribe();
    host.send(Command::SetAutoScan { enabled: false })?;
    host.send(Command::GetState)?;
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        if Instant::now() >= until {
            return Err("Scanner readiness timeout".into());
        }
        let event: Event = serde_json::from_str(&events.recv_timeout(Duration::from_secs(5))?)?;
        if let Event::SettingsChanged { settings } = event
            && settings.get("profileReady").map(String::as_str) == Some("true")
        {
            break;
        }
    }
    host.set_web_enabled(true)?;
    assert!(host.web_running());
    host.set_mcp_enabled(true)?;
    assert!(host.mcp_running());
    let init = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":local_http::MCP_VERSION,"capabilities":{},"clientInfo":{"name":"local-verification","version":"1"}}});
    let answer =
        local_http::mcp_bridge(&serde_json::to_vec(&init)?)?.ok_or("Missing initialize reply")?;
    assert_eq!(answer["result"]["protocolVersion"], local_http::MCP_VERSION);
    let mut report = Vec::new();
    for index in 1..=count {
        host.send(Command::CaptureSide { side, document: None })?;
        let started = Instant::now();
        let mut recognizing = false;
        let scan = loop {
            let event: Event =
                serde_json::from_str(&events.recv_timeout(Duration::from_secs(90))?)?;
            match event {
                Event::ScanProgress {
                    stage: passport_scanner_protocol::Stage::Recognising,
                    ..
                } => recognizing = true,
                Event::ScanCompleted { scan } => break scan,
                Event::Failed { code, .. } if code == "scanFailed" => {
                    return Err("Hardware scan failed".into());
                }
                _ => {}
            }
        };
        assert!(recognizing);
        let prefix = format!("{index:03}");
        std::fs::write(out.join(format!("{prefix}.json")), serde_json::to_vec_pretty(&scan)?)?;
        for image in &scan.images {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(image.url.split_once(',').ok_or("Invalid image URL")?.1)?;
            let name = match image.source {
                passport_scanner_protocol::Source::White => "white",
                passport_scanner_protocol::Source::Infrared => "infrared",
                _ => "other",
            };
            std::fs::write(out.join(format!("{prefix}-{name}.jpg")), bytes)?;
        }
        let checks: Vec<_> = scan.fields.iter().filter_map(|field| field.check).collect();
        let mrz_valid = scan.mrz.len() == 3
            && scan.mrz_check == Some(true)
            && checks.len() >= 3
            && checks.iter().all(|pass| *pass);
        let expected_result = if side == passport_scanner_protocol::Side::Front {
            scan.mrz.is_empty() && scan.fields.is_empty() && scan.mrz_check.is_none()
        } else {
            mrz_valid
        };
        let request = serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_mrz","arguments":{"document_id":scan.document}}});
        let mrz =
            local_http::mcp_bridge(&serde_json::to_vec(&request)?)?.ok_or("Missing tool reply")?;
        assert!(mrz.get("error").is_none());
        if side == passport_scanner_protocol::Side::Front {
            assert_eq!(mrz["result"]["structuredContent"]["error"]["code"], "mrz_not_found");
        } else if mrz_valid {
            assert!(
                mrz["result"]["structuredContent"]["rawLines"] == serde_json::to_value(&scan.mrz)?,
                "MCP returned different MRZ data"
            );
            assert!(
                mrz["result"]["structuredContent"]["fields"] == serde_json::to_value(&scan.fields)?,
                "MCP returned different fields"
            );
        }
        assert!(host.export_pdf(&scan.document, &out.join("incomplete.pdf")).is_err());
        let row = serde_json::json!({"scan":index,"mrz_lines":scan.mrz.len(),"checks":checks,"composite":scan.mrz_check,"mrz_valid":mrz_valid,"expected_result":expected_result,"seconds":started.elapsed().as_secs_f32(),"side":side});
        println!("{row}");
        report.push(row);
    }
    host.set_web_enabled(false)?;
    assert!(!host.web_running());
    assert!(std::net::TcpStream::connect(local_http::WEB_ADDRESS).is_err());
    assert!(std::net::TcpStream::connect(local_http::API_ADDRESS).is_ok());
    host.set_web_enabled(true)?;
    assert!(host.web_running());
    host.set_web_enabled(false)?;
    host.set_mcp_enabled(false)?;
    assert!(!host.mcp_running());
    assert!(std::net::TcpStream::connect(local_http::MCP_ADDRESS).is_err());
    drop(host);
    assert!(std::net::TcpStream::connect(local_http::API_ADDRESS).is_err());
    let all_expected = report.iter().all(|row| row["expected_result"] == true);
    let summary = serde_json::json!({"scans":report,"all_expected_results":all_expected,"physical_side":side,"web_start_stop_restart":true,"mcp_initialize_and_read":true,"incomplete_pdf_refused":true,"host_cleanup":true});
    std::fs::write(out.join("report.json"), serde_json::to_vec_pretty(&summary)?)?;
    println!(
        "Session verification complete; {count} scans; all expected recognition results: {all_expected}."
    );
    Ok(())
}
