//! Repeat captures and retain evidence for each attempt.
//!
//! Raw and processed frames from every requested light source are saved beside
//! manifest.json. Capture/processing success is separate from visual quality:
//! histogram and crop measurements are descriptive, not an acceptance test.
//!
//! cargo run --release -p passport-scanner-app --bin passport-scanner-survey -- 20 scans/survey
//! Optional flags: white, nocontrols, reopen, reinit, id1-reference,
//! profile=general|identity|identity-bright.
//! The optional ID-1 ratio is a reference for suitable cards, not passports.

use passport_scanner_core::{Frame, PixelFormat};
use passport_scanner_imaging::{levels, luma};
use serde_json::{Value, json};
use std::path::Path;

type SurveyResult<T> = Result<T, Box<dyn std::error::Error>>;

fn measure(frame: &Frame, id1_reference: bool) -> SurveyResult<Value> {
    let image = image::load_from_memory(&frame.data)?.to_rgb8();
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Err("empty image".into());
    }
    let mut histogram = [0u64; 256];
    let (mut bright, mut dark) = ([0u64; 3], [0u64; 3]);
    let (mut bright_count, mut dark_count) = (0u64, 0u64);
    for p in image.pixels() {
        let y = luma(p.0);
        histogram[y as usize] += 1;
        let (sums, count) =
            if y >= 170 { (&mut bright, &mut bright_count) } else { (&mut dark, &mut dark_count) };
        *count += 1;
        for (sum, value) in sums.iter_mut().zip(p.0) {
            *sum += u64::from(value);
        }
    }
    let mean = |sums: [u64; 3], n: u64| -> Option<[f64; 3]> {
        (n > 0).then(|| sums.map(|s| s as f64 / n as f64))
    };
    let channel_spread = |means: Option<[f64; 3]>| {
        means.map(|c| {
            c.iter().copied().fold(f64::MIN, f64::max) - c.iter().copied().fold(f64::MAX, f64::min)
        })
    };
    let bright_mean = mean(bright, bright_count);
    let dark_mean = mean(dark, dark_count);
    let ratio = f64::from(width) / f64::from(height);
    let spread = levels::black_white(&histogram, levels::Cuts::default())
        .map_or(0, |p| p.white.saturating_sub(p.black));
    let signal_fraction =
        histogram[32..].iter().sum::<u64>() as f64 / (u64::from(width) * u64::from(height)) as f64;
    Ok(json!({
        "width": width, "height": height, "aspect_ratio": ratio,
        "id1_reference_difference_percent": id1_reference.then(|| (ratio / (85.6 / 54.0) - 1.0) * 100.0),
        "luma_spread": spread, "bright_threshold_luma": 170,
        "fraction_luma_at_least_32": signal_fraction,
        "bright_pixels": bright_count, "dark_pixels": dark_count,
        "bright_mean_rgb": bright_mean, "dark_mean_rgb": dark_mean,
        "bright_channel_spread": channel_spread(bright_mean),
        "dark_channel_spread": channel_spread(dark_mean)
    }))
}

/// A rejection screen, never a document detector or color-quality pass.
/// The observed empty/dark white frames had roughly 0.13% of pixels >=32;
/// visible document captures, including a very dark exposure, exceeded 20%.
/// This deliberately low 1% threshold rejects only unusably weak signal.
fn quality_eligibility(artifacts: &[Value]) -> Value {
    let fraction = artifacts
        .iter()
        .find(|a| a["stage"] == "raw" && a["source"] == "white")
        .and_then(|a| a["metrics"]["fraction_luma_at_least_32"].as_f64());
    let status = match fraction {
        Some(value) if value < 0.01 => "rejected_insufficient_signal",
        Some(_) => "needs_visual_review",
        None => "unavailable",
    };
    json!({
        "status": status, "raw_white_signal_fraction": fraction,
        "minimum_signal_fraction": 0.01, "luma_threshold": 32,
        "document_presence": "not_verified", "quality_pass": null,
        "note": "Insufficient signal can mean empty platen, severe underexposure or obscuration. Passing this screen does not confirm a document or acceptable quality."
    })
}

fn save_frames(
    out: &Path,
    run: usize,
    stage: &str,
    frames: &[Frame],
    id1_reference: bool,
    artifacts: &mut Vec<Value>,
) -> SurveyResult<()> {
    for frame in frames {
        let extension = if frame.format == PixelFormat::Jpeg { "jpg" } else { "bin" };
        let filename = format!("{run:03}-{stage}-{}.{extension}", frame.source);
        std::fs::write(out.join(&filename), &frame.data)?;
        let (metrics, metrics_error) = match measure(frame, id1_reference) {
            Ok(metrics) => (metrics, None),
            Err(error) => (Value::Null, Some(error.to_string())),
        };
        artifacts.push(json!({
            "stage": stage, "source": frame.source.to_string(), "path": filename,
            "bytes": frame.data.len(), "frame_width": frame.width, "frame_height": frame.height,
            "pixel_format": format!("{:?}", frame.format),
            "metrics": metrics, "metrics_error": metrics_error
        }));
    }
    Ok(())
}

fn update_counts(manifest: &mut Value) {
    let records = manifest["runs"].as_array().unwrap();
    let attempted = records.len();
    let captured = records.iter().filter(|r| r["capture_succeeded"] == true).count();
    let succeeded = records.iter().filter(|r| r["succeeded"] == true).count();
    let processed = records.iter().filter(|r| r["processing_succeeded"] == true).count();
    let failed = records.iter().filter(|r| r["status"] == "failed").count();
    let review_candidates = records
        .iter()
        .filter(|r| {
            r["succeeded"] == true && r["quality_eligibility"]["status"] == "needs_visual_review"
        })
        .count();
    let signal_rejections = records
        .iter()
        .filter(|r| r["quality_eligibility"]["status"] == "rejected_insufficient_signal")
        .count();
    manifest["attempted"] = json!(attempted);
    manifest["capture_succeeded"] = json!(captured);
    manifest["succeeded"] = json!(succeeded);
    manifest["processing_succeeded"] = json!(processed);
    manifest["failed"] = json!(failed);
    manifest["visual_review_candidates"] = json!(review_candidates);
    manifest["insufficient_signal_rejections"] = json!(signal_rejections);
}

fn write_manifest(out: &Path, manifest: &mut Value) -> SurveyResult<()> {
    update_counts(manifest);
    std::fs::write(out.join("manifest.json"), serde_json::to_vec_pretty(manifest)?)?;
    Ok(())
}

#[cfg(not(windows))]
fn main() -> SurveyResult<()> {
    Err("This build has no transport for the current platform.".into())
}

#[cfg(windows)]
fn main() -> SurveyResult<()> {
    use passport_scanner_core::LightSource;
    use std::{
        path::PathBuf,
        time::{Instant, SystemTime, UNIX_EPOCH},
    };

    let args: Vec<String> = std::env::args().collect();
    let runs: usize = args.get(1).map(|a| a.parse()).transpose()?.unwrap_or(100);
    if runs == 0 {
        return Err("at least one scan is required".into());
    }
    let out: PathBuf = args.get(2).cloned().unwrap_or_else(|| "scans/survey".into()).into();
    let flag = |name: &str| args.iter().skip(3).any(|a| a == name);
    let selection = passport_scanner_app::preferences::parse_profile(
        args.iter().skip(3).find_map(|a| a.strip_prefix("profile=")).unwrap_or("general"),
    )?;
    let (white_only, no_controls, reopen, reinit, id1_reference) =
        (flag("white"), flag("nocontrols"), flag("reopen"), flag("reinit"), flag("id1-reference"));
    std::fs::create_dir_all(&out)?;
    if out.join("manifest.json").exists() {
        return Err("output already contains manifest.json; use a new survey directory to retain prior evidence".into());
    }
    let mut manifest = json!({
        "schema_version": 2,
        "started_unix_ms": SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
        "requested": runs, "status": "starting", "quality_assessment": "not_evaluated",
        "success_definition": "all requested sources captured, processed, saved and decoded, with sufficient raw white signal for visual review; not a document-presence or visual-quality pass",
        "options": { "white_only": white_only, "no_controls": no_controls, "reopen": reopen,
            "reinit": reinit, "id1_reference": id1_reference,
            "capture_profile": passport_scanner_app::preferences::profile_name(selection) },
        "metrics_note": "Bright/dark pixel groups are threshold measurements, not identified paper/ink or a color accuracy test. Optional ID-1 comparison is applicable only to matching card formats.",
        "errors": [], "runs": []
    });
    write_manifest(&out, &mut manifest)?;
    let setup = (|| -> SurveyResult<_> {
        let (mut scanner, model, loaded) = passport_scanner_app::open_with_selection(selection)?;
        if no_controls {
            scanner.set_white_controls(Vec::new());
        }
        Ok((scanner, model, loaded))
    })();
    let (scanner, model, loaded) = match setup {
        Ok(setup) => setup,
        Err(error) => {
            manifest["status"] = json!("failed");
            manifest["errors"] = json!([{ "stage": "setup", "message": error.to_string() }]);
            write_manifest(&out, &mut manifest)?;
            return Err(error);
        }
    };
    manifest["calibration_source"] = json!(loaded.source);
    manifest["white_exposure_trim"] = json!(scanner.calibration().white_exposure.map(|v| v.value));
    let shading = loaded.shading;
    let mut scanner = Some(scanner);
    let sources = if white_only { vec![LightSource::White] } else { model.descriptor().sources };
    manifest["model"] = json!(model.name());
    manifest["calibration_found"] = json!(shading.is_some());
    manifest["requested_sources"] =
        json!(sources.iter().map(ToString::to_string).collect::<Vec<_>>());
    manifest["status"] = json!("running");

    for run in 1..=runs {
        manifest["runs"].as_array_mut().unwrap().push(json!({
            "run": run, "status": "running", "capture_succeeded": false,
            "succeeded": false, "processing_succeeded": false,
            "quality_eligibility": { "status": "unavailable", "quality_pass": null },
            "errors": [], "artifacts": []
        }));
        write_manifest(&out, &mut manifest)?;
        let started = Instant::now();
        let mut artifacts = Vec::new();
        let mut stage = "initialise";
        let mut capture_succeeded = false;
        let mut processing_succeeded = false;
        let result = (|| -> SurveyResult<()> {
            if (reopen && run > 1) || scanner.is_none() {
                // Release the current camera before opening its replacement.
                // A failed open leaves None so the next attempt can recover.
                drop(scanner.take());
                scanner = Some(passport_scanner_app::open_with_selection(selection)?.0);
                if no_controls {
                    scanner.as_mut().unwrap().set_white_controls(Vec::new());
                }
            }
            let scanner = scanner.as_mut().ok_or("scanner is unavailable")?;
            if reinit && run > 1 {
                scanner.initialise()?;
                passport_scanner_app::select_capture(scanner, model, selection)?;
            }
            stage = "capture";
            let mut frames = scanner.scan(&sources)?;
            capture_succeeded =
                sources.iter().all(|source| frames.iter().any(|frame| frame.source == *source));
            stage = "save_raw";
            save_frames(&out, run, "raw", &frames, id1_reference, &mut artifacts)?;
            stage = "capture_validation";
            for source in &sources {
                if !frames.iter().any(|frame| frame.source == *source) {
                    return Err(format!("missing requested {source} frame").into());
                }
            }
            stage = "processing";
            passport_scanner_app::enhance(&mut frames, shading.as_ref())?;
            stage = "save_processed";
            save_frames(&out, run, "processed", &frames, id1_reference, &mut artifacts)?;
            stage = "decode_processed";
            for source in &sources {
                let frame = frames
                    .iter()
                    .find(|f| f.source == *source)
                    .ok_or_else(|| format!("missing processed {source} frame"))?;
                measure(frame, id1_reference)?;
            }
            processing_succeeded = true;
            stage = "quality_eligibility";
            match quality_eligibility(&artifacts)["status"].as_str() {
                Some("needs_visual_review") => {}
                Some("rejected_insufficient_signal") => {
                    return Err("raw white frame has insufficient signal: empty platen, severe underexposure or obscuration; excluded from document-quality trials".into());
                }
                _ => return Err("raw white signal could not be assessed".into()),
            }
            Ok(())
        })();
        let record = &mut manifest["runs"][run - 1];
        record["elapsed_ms"] = json!(started.elapsed().as_millis());
        record["capture_succeeded"] = json!(capture_succeeded);
        record["processing_succeeded"] = json!(processing_succeeded);
        record["quality_eligibility"] = quality_eligibility(&artifacts);
        record["artifacts"] = json!(artifacts);
        match result {
            Ok(()) => {
                record["status"] = json!("succeeded");
                record["succeeded"] = json!(true);
                println!(
                    "{run:>4}/{runs}: captured, processed and saved in {} ms (quality not evaluated)",
                    started.elapsed().as_millis()
                );
            }
            Err(error) => {
                record["status"] = json!("failed");
                record["errors"] = json!([{ "stage": stage, "message": error.to_string() }]);
                eprintln!("{run:>4}/{runs}: {stage} failed: {error}");
            }
        }
        write_manifest(&out, &mut manifest)?;
    }
    let failed = manifest["failed"].as_u64().unwrap();
    manifest["status"] = json!(if failed == 0 { "completed" } else { "failed" });
    write_manifest(&out, &mut manifest)?;
    println!(
        "{} attempted, {} captured, {} processed/saved, {} candidates for visual review, {failed} failed. Evidence: {}",
        manifest["attempted"],
        manifest["capture_succeeded"],
        manifest["processing_succeeded"],
        manifest["visual_review_candidates"],
        out.join("manifest.json").display()
    );
    if failed != 0 {
        return Err(format!("{failed} survey attempts failed").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weak_signal_is_rejected_without_claiming_an_empty_platen() {
        let artifacts = vec![json!({ "stage": "raw", "source": "white",
            "metrics": { "fraction_luma_at_least_32": 0.0013 } })];
        let eligibility = quality_eligibility(&artifacts);
        assert_eq!(eligibility["status"], "rejected_insufficient_signal");
        assert_eq!(eligibility["document_presence"], "not_verified");
        assert!(eligibility["quality_pass"].is_null());
    }

    #[test]
    fn bright_frame_is_only_a_review_candidate() {
        let artifacts = vec![json!({ "stage": "raw", "source": "white",
            "metrics": { "fraction_luma_at_least_32": 1.0 } })];
        let eligibility = quality_eligibility(&artifacts);
        assert_eq!(eligibility["status"], "needs_visual_review");
        assert_eq!(eligibility["document_presence"], "not_verified");
        assert!(eligibility["quality_pass"].is_null());
        assert_eq!(quality_eligibility(&[])["status"], "unavailable");
    }

    #[test]
    fn counts_separate_capture_processing_failure_and_incomplete_attempt() {
        let mut manifest = json!({ "runs": [
            { "status": "succeeded", "capture_succeeded": true, "succeeded": true },
            { "status": "failed", "capture_succeeded": true, "succeeded": false },
            { "status": "failed", "capture_succeeded": false, "succeeded": false },
            { "status": "running", "capture_succeeded": false, "succeeded": false }
        ] });
        update_counts(&mut manifest);
        assert_eq!(manifest["attempted"], 4);
        assert_eq!(manifest["capture_succeeded"], 2);
        assert_eq!(manifest["succeeded"], 1);
        assert_eq!(manifest["failed"], 2);
    }

    #[test]
    fn both_sources_are_retained_with_stage_and_metrics() {
        use passport_scanner_core::LightSource;
        let out = std::env::temp_dir()
            .join(format!("passport-scanner-survey-test-{}", std::process::id()));
        std::fs::create_dir_all(&out).unwrap();
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode(&[220u8; 12], 2, 2, image::ExtendedColorType::Rgb8)
            .unwrap();
        let frames: Vec<_> = [LightSource::White, LightSource::Infrared]
            .into_iter()
            .map(|source| Frame {
                width: 2,
                height: 2,
                source,
                format: PixelFormat::Jpeg,
                data: jpeg.clone(),
            })
            .collect();
        let mut artifacts = Vec::new();
        for stage in ["raw", "processed"] {
            save_frames(&out, 1, stage, &frames, false, &mut artifacts).unwrap();
        }
        assert_eq!(artifacts.len(), 4);
        for artifact in artifacts {
            let path = out.join(artifact["path"].as_str().unwrap());
            assert_eq!(std::fs::read(&path).unwrap(), jpeg);
            assert_eq!(artifact["metrics"]["width"], 2);
            assert!(artifact["metrics"]["id1_reference_difference_percent"].is_null());
            assert!(artifact["metrics"]["dark_mean_rgb"].is_null());
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir(out).unwrap();
    }
}
