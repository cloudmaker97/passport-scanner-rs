//! Offline, privacy-safe orientation check. Never opens scanner hardware.
//! Usage: review_orientation WHITE.jpg INFRARED.jpg NEW_PRIVATE_OUTPUT_DIRECTORY
//! The input pair must show the same upside-down back (180-degree correction).
//! PDFs deliberately duplicate that back on both labelled sides for export testing;
//! they are diagnostic artifacts, not completed document exports.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{ImageDecoder, codecs::jpeg::JpegDecoder};
use passport_scanner_app::{export, orientation, read_document_oriented};
use passport_scanner_core::{FieldKind, Frame, LightSource, PixelFormat};
use passport_scanner_protocol::{Image, Scan, Side, Source};
use std::{io::Cursor, path::PathBuf, time::Instant};

fn load(path: PathBuf, source: LightSource) -> Result<Frame, Box<dyn std::error::Error>> {
    let data = std::fs::read(path)?;
    let decoder = JpegDecoder::new(Cursor::new(&data))?;
    let (width, height) = decoder.dimensions();
    Ok(Frame { width, height, format: PixelFormat::Jpeg, source, data })
}

fn verified(reading: &passport_scanner_recognize::Recognition) -> bool {
    reading.document.get(FieldKind::MrzLine).is_some_and(|field| field.checksum_ok == Some(true))
        && reading.document.checksums_valid()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let white = PathBuf::from(args.next().ok_or("expected white JPEG")?);
    let infrared = PathBuf::from(args.next().ok_or("expected infrared JPEG")?);
    let output = PathBuf::from(args.next().ok_or("expected new private output directory")?);
    if args.next().is_some() || output.exists() {
        return Err("choose a new output directory and supply exactly three arguments".into());
    }
    let originals = [load(white, LightSource::White)?, load(infrared, LightSource::Infrared)?];
    std::fs::create_dir_all(&output)?;
    let mut report = Vec::new();
    for (input_rotation, expected_correction) in [(0, 180), (90, 90), (180, 0), (-90, -90)] {
        let started = Instant::now();
        let mut frames = originals.clone();
        orientation::orient_frames(&mut frames, input_rotation)?;
        let before = read_document_oriented(&frames)?;
        if before.rotation != Some(expected_correction) || !verified(&before) {
            return Err(format!("input_rotation={input_rotation}: expected verified correction {expected_correction}, got {:?}, verified={}", before.rotation, verified(&before)).into());
        }
        orientation::orient_frames(&mut frames, expected_correction)?;
        let after = read_document_oriented(&frames)?;
        if after.rotation != Some(0)
            || !verified(&after)
            || after.document.fields != before.document.fields
        {
            return Err(format!("input_rotation={input_rotation}: corrected recognition is not verified, upright, and stable").into());
        }
        let corrected_bytes: Vec<_> = frames.iter().map(|frame| frame.data.clone()).collect();
        orientation::orient_frames(&mut frames, 0)?;
        if frames.iter().zip(&corrected_bytes).any(|(frame, bytes)| frame.data != *bytes) {
            return Err("zero rotation changed JPEG bytes".into());
        }
        let directory = output.join(format!("input-{input_rotation}"));
        std::fs::create_dir(&directory)?;
        let mut images = Vec::new();
        for (index, frame) in frames.iter().enumerate() {
            let (width, height) = orientation::display_dimensions(frame)?;
            let name = if index == 0 { "white.jpg" } else { "infrared.jpg" };
            std::fs::write(directory.join(name), &frame.data)?;
            images.push(Image {
                source: if index == 0 { Source::White } else { Source::Infrared },
                url: format!("data:image/jpeg;base64,{}", STANDARD.encode(&frame.data)),
                width,
                height,
            });
        }
        let scans = [Side::Front, Side::Back]
            .into_iter()
            .enumerate()
            .map(|(index, side)| Scan {
                id: format!("diagnostic-{index}"),
                document: "diagnostic-duplicated-back".into(),
                side,
                at: "2026-01-01T00:00:00Z".into(),
                format: None,
                mrz: vec![],
                mrz_check: None,
                fields: vec![],
                images: images.clone(),
            })
            .collect::<Vec<_>>();
        let pdf = export::document_pdf(&scans, "diagnostic-duplicated-back")?;
        for frame in &frames {
            if pdf.windows(frame.data.len()).filter(|bytes| *bytes == frame.data).count() != 2 {
                return Err("PDF did not embed each corrected JPEG twice, byte-for-byte".into());
            }
        }
        std::fs::write(directory.join("diagnostic-duplicated-back.pdf"), &pdf)?;
        let result = serde_json::json!({
            "input_rotation": input_rotation,
            "correction": before.rotation,
            "second_correction": after.rotation,
            "verified": true,
            "fields_stable": true,
            "zero_rotation_byte_identical": true,
            "pdf_jpeg_bytes_exact": true,
            "elapsed_ms": started.elapsed().as_millis(),
            "pdf_bytes": pdf.len(),
        });
        println!("{result}");
        report.push(result);
        std::fs::write(output.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    }
    Ok(())
}
