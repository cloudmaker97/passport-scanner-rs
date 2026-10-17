//! Conservative whole-document orientation when no verified MRZ is available.
//!
//! OSD is accepted only with strong confidence. Sparse-text fallback compares
//! all four rotations without exposing text or populating document fields.
//! Ambiguous or text-poor images stay unchanged.

use crate::ocr::Tesseract;
use image::{GrayImage, imageops};
use passport_scanner_core::{Error, Result};

const MAX_EDGE: u32 = 1400;
const ANGLES: [i16; 4] = [0, 180, -90, 90];

pub(crate) fn detect(engine: &Tesseract, image: &GrayImage) -> Result<Option<i16>> {
    if image.width() == 0 || image.height() == 0 {
        return Ok(None);
    }
    let largest = image.width().max(image.height());
    let small = if largest > MAX_EDGE {
        imageops::resize(
            image,
            ((u64::from(image.width()) * u64::from(MAX_EDGE)) / u64::from(largest)).max(1) as u32,
            ((u64::from(image.height()) * u64::from(MAX_EDGE)) / u64::from(largest)).max(1) as u32,
            imageops::FilterType::Lanczos3,
        )
    } else {
        image.clone()
    };

    if let Some(angle) = engine.orientation_text(&small, true)?.as_deref().and_then(osd_angle) {
        tracing::debug!(
            rotation_clockwise_degrees = angle,
            "Document orientation accepted from OSD"
        );
        return Ok(Some(angle));
    }

    let mut candidates = Vec::with_capacity(4);
    for pair in ANGLES.chunks(2) {
        let results = std::thread::scope(|scope| {
            let jobs: Vec<_> = pair
                .iter()
                .map(|&angle| {
                    let small = &small;
                    scope.spawn(move || {
                        let rotated = match angle {
                            180 => imageops::rotate180(small),
                            -90 => imageops::rotate270(small),
                            90 => imageops::rotate90(small),
                            _ => small.clone(),
                        };
                        let start = std::time::Instant::now();
                        let Some(text) = engine.orientation_text(&rotated, false)? else {
                            return Ok((angle, None));
                        };
                        let score = text_score(&text);
                        tracing::debug!(
                            rotation_clockwise_degrees = angle,
                            confident_words = score.words,
                            confident_characters = score.characters,
                            elapsed_ms = start.elapsed().as_millis(),
                            "Document orientation candidate completed"
                        );
                        Ok((angle, Some(score)))
                    })
                })
                .collect();
            jobs.into_iter()
                .map(|job| {
                    job.join().unwrap_or_else(|_| {
                        Err(Error::Rejected("orientation worker panicked".into()))
                    })
                })
                .collect::<Vec<_>>()
        });
        for (angle, score) in results.into_iter().collect::<Result<Vec<_>>>()? {
            // Missing comparisons cannot support a confidence margin.
            let Some(score) = score else {
                return Ok(None);
            };
            candidates.push((angle, score));
        }
    }
    Ok(select(&candidates))
}

fn osd_angle(text: &str) -> Option<i16> {
    let value =
        |prefix: &str| text.lines().find_map(|line| line.strip_prefix(prefix).map(str::trim));
    let confidence: f64 = value("Orientation confidence:")?.parse().ok()?;
    if !confidence.is_finite() || confidence < 15.0 {
        return None;
    }
    match value("Rotate:")?.parse::<i16>().ok()? {
        0 => Some(0),
        90 => Some(90),
        180 => Some(180),
        270 => Some(-90),
        _ => None,
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Score {
    words: usize,
    characters: usize,
    weight: f64,
}

fn text_score(tsv: &str) -> Score {
    let mut score = Score::default();
    for line in tsv.lines() {
        // Tesseract TSV is tab-delimited, not CSV-quoted: quote characters can
        // be literal OCR output and must not consume subsequent rows.
        let fields: Vec<_> = line.splitn(12, '\t').collect();
        if fields.len() != 12 || fields[0] != "5" {
            continue;
        }
        let Ok(confidence) = fields[10].parse::<f64>() else {
            continue;
        };
        let characters = fields[11].chars().filter(|c| c.is_alphanumeric()).count();
        let total = fields[11].chars().count();
        if !(70.0..=100.0).contains(&confidence)
            || !(3..=32).contains(&characters)
            || characters * 2 < total
        {
            continue;
        }
        score.words += 1;
        score.characters += characters;
        score.weight += confidence * characters as f64 / 100.0;
    }
    score
}

fn select(candidates: &[(i16, Score)]) -> Option<i16> {
    let mut ranked = candidates.to_vec();
    ranked.sort_by(|a, b| b.1.weight.total_cmp(&a.1.weight));
    let (angle, best) = *ranked.first()?;
    let runner = ranked.get(1).map_or(0.0, |(_, score)| score.weight);
    // Enough independent text plus both absolute and relative separation.
    // These are conservative heuristics, not a probability of correctness.
    (best.words >= 4
        && best.characters >= 20
        && best.weight >= 20.0
        && best.weight - runner >= 15.0
        && best.weight >= runner * 1.8)
        .then_some(angle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Engine;

    #[test]
    fn osd_requires_confidence_and_maps_correction_direction() {
        for (reported, expected) in [(0, 0), (90, 90), (180, 180), (270, -90)] {
            assert_eq!(
                osd_angle(&format!("Rotate: {reported}\nOrientation confidence: 15.2\n")),
                Some(expected)
            );
        }
        for confidence in ["0.8", "NaN", "inf", "-5"] {
            assert_eq!(
                osd_angle(&format!("Rotate: 180\nOrientation confidence: {confidence}")),
                None
            );
        }
        assert_eq!(osd_angle("Rotate: 45\nOrientation confidence: 99"), None);
    }

    fn row(confidence: &str, text: &str) -> String {
        format!("5\t1\t1\t1\t1\t1\t1\t1\t40\t12\t{confidence}\t{text}\n")
    }

    #[test]
    fn scores_use_only_confident_words_and_do_not_treat_quotes_as_csv() {
        let tsv = row("96", "DOCUMENT")
            + &row("81", "\"SAMPLE")
            + &row("20", "IGNORE")
            + &row("NaN", "IGNORE")
            + &row("95", "!!!")
            + &row("90", "A");
        let score = text_score(&tsv);
        assert_eq!((score.words, score.characters), (2, 14));
        assert!((score.weight - 12.54).abs() < 0.001);
    }

    #[test]
    fn ambiguous_and_sparse_text_does_not_rotate() {
        let strong = Score { words: 8, characters: 50, weight: 45.0 };
        assert_eq!(select(&[(0, strong), (180, strong)]), None);
        assert_eq!(select(&[(90, Score { weight: 40.0, ..strong }), (0, strong)]), None);
        assert_eq!(select(&[(0, Score { words: 1, ..strong })]), None);
        assert_eq!(select(&[(180, strong), (0, Score::default())]), Some(180));
    }

    /// Local opt-in fixture; no identity text is stored in source or output.
    #[test]
    #[ignore = "requires local Tesseract and PASSPORT_SCANNER_ORIENTATION_IMAGE pointing to an upright document"]
    fn local_front_all_four_rotations() {
        let path = std::env::var_os("PASSPORT_SCANNER_ORIENTATION_IMAGE")
            .expect("local image path required");
        let upright = image::open(path).unwrap().to_luma8();
        let engine = Tesseract::default();
        for (input, correction) in [
            (upright.clone(), 0),
            (imageops::rotate90(&upright), -90),
            (imageops::rotate180(&upright), 180),
            (imageops::rotate270(&upright), 90),
        ] {
            let start = std::time::Instant::now();
            let detected = engine.orientation(&input).unwrap();
            println!(
                "orientation expected={correction}, detected={detected:?}, elapsed_ms={}",
                start.elapsed().as_millis()
            );
            assert_eq!(detected, Some(correction));
        }
        let blank = GrayImage::from_pixel(1000, 600, image::Luma([245]));
        assert_eq!(engine.orientation(&blank).unwrap(), None);
    }
}
