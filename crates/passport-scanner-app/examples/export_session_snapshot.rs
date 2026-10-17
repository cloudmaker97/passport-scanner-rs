//! Offline PDF diagnostic for a saved Vec<Event>; never opens scanner hardware.
//! Output is private document imagery: choose a directory under ignored scans/.
use passport_scanner_protocol::Event;
use std::{path::PathBuf, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let input = PathBuf::from(arguments.next().ok_or("expected session JSON path")?);
    let output = PathBuf::from(arguments.next().ok_or("expected output directory")?);
    if arguments.next().is_some() {
        return Err("too many arguments".into());
    }
    let bytes = std::fs::read(input)?;
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    let events: Vec<Event> = serde_json::from_slice(bytes)?;
    let scans = events
        .into_iter()
        .filter_map(|event| match event {
            Event::State { scans, .. } => Some(scans),
            _ => None,
        })
        .next_back()
        .ok_or("snapshot contains no State event")?;
    let mut documents = Vec::new();
    for scan in &scans {
        if !documents.contains(&scan.document.as_str()) {
            documents.push(&scan.document);
        }
    }
    std::fs::create_dir_all(&output)?;
    let mut failures = 0;
    for (index, document) in documents.iter().enumerate() {
        let path = output.join(format!("document-{:03}.pdf", index + 1));
        if path.exists() {
            return Err("diagnostic output already exists; choose a fresh directory".into());
        }
        let start = Instant::now();
        match passport_scanner_app::export::write_document_pdf(&scans, document, &path) {
            Ok(()) => println!(
                "document_index={} result=ok bytes={} elapsed_ms={}",
                index + 1,
                std::fs::metadata(path)?.len(),
                start.elapsed().as_millis()
            ),
            Err(error) => {
                failures += 1;
                eprintln!(
                    "document_index={} result=failed elapsed_ms={} error={error}",
                    index + 1,
                    start.elapsed().as_millis()
                );
            }
        }
    }
    if failures > 0 {
        return Err(format!("{failures} document exports failed").into());
    }
    Ok(())
}
