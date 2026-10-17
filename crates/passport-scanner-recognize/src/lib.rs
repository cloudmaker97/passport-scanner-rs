//! Finding and reading the machine-readable zone on a captured document.
//!
//! The work splits into three parts, and only the last needs anything
//! installed:
//!
//! - [`locate`] finds the zone strip on a document that has been located. Pure
//!   geometry.
//! - [`text`] turns whatever an engine said into zone lines. Pure strings.
//! - [`MrzReader`] joins them to an OCR engine and
//!   [`passport_scanner_mrz`]'s parser.
//!
//! Keeping the first two pure means the awkward decisions - what counts as the
//! document, what counts as a zone line - are tested without a camera or an
//! engine anywhere near them.

#![forbid(unsafe_code)]

pub mod deskew;
#[cfg(test)]
mod deskew_tests;
pub mod locate;
pub mod ocr;
pub mod orientation;
pub mod text;

use image::{DynamicImage, GenericImageView, ImageDecoder, ImageReader};
use passport_scanner_core::{
    Document, DocumentRecognizer, Error, Frame, LightSource, PixelFormat, Result,
};
use std::io::Cursor;

pub use ocr::{Engine, Tesseract};

/// OCR-only enlargement. Preserve grayscale strokes: a global binary split
/// can erase parts of the MRZ under uneven illumination and confuses fillers.
pub const SCALE: u32 = 2;

/// What one attempt at reading a frame produced.
#[derive(Debug, Clone)]
pub struct Reading {
    /// The part of the frame handed to the engine.
    pub strip: passport_scanner_core::Rect,
    /// What the engine said, verbatim.
    pub text: String,
    /// The zone, if the text held a complete one.
    pub lines: Option<Vec<String>>,
}

/// Verified document fields and an independently determined display correction.
#[derive(Debug, Default)]
pub struct Recognition {
    pub document: Document,
    /// Clockwise rotation relative to the image's existing EXIF orientation.
    /// None means the available evidence does not determine an upright view.
    pub rotation: Option<i16>,
}

/// Reads the machine-readable zone off a captured frame.
#[derive(Debug)]
pub struct MrzReader<E> {
    engine: E,
    /// How bright a pixel must be to count as document rather than platen.
    pub threshold: u8,
}

impl<E: Engine> MrzReader<E> {
    /// Build a reader over an OCR engine.
    pub fn new(engine: E) -> Self {
        Self { engine, threshold: 110 }
    }

    /// Read the zone from one frame.
    ///
    /// # Errors
    /// Fails if the frame cannot be decoded, no document is found, or the
    /// engine does not run.
    pub fn read_frame(&self, frame: &Frame) -> Result<Vec<String>> {
        let reading = self.read_frame_raw(frame)?;
        reading
            .lines
            .ok_or_else(|| Error::Malformed("no complete zone in the recognised text".to_owned()))
    }

    /// Read the zone, keeping what the engine actually said.
    ///
    /// When a zone that is plainly legible does not come out, the question is
    /// always which step lost it - the engine, or the repair that follows it.
    /// Returning both answers it instead of leaving it to be guessed at.
    ///
    /// # Errors
    /// Fails if the frame cannot be decoded, no document is found, or the
    /// engine does not run. Text that yields no zone is not an error: it comes
    /// back with `lines` empty.
    pub fn read_frame_raw(&self, frame: &Frame) -> Result<Reading> {
        self.read_frame_oriented(frame).map(|(reading, _)| reading)
    }

    /// Read upright, then upside down, then rotated left, then right.
    /// Returns the successful clockwise rotation in degrees (0, 180, -90, 90).
    /// The reported strip is mapped back to the original frame coordinates;
    /// the input pixels are never changed. The angle may drive a UI preview.
    pub fn read_frame_oriented(&self, frame: &Frame) -> Result<(Reading, i16)> {
        let image = decode(frame)?;
        let mut fallback = None;
        let mut last = None;
        for angle in [0, 180, -90, 90] {
            let started = std::time::Instant::now();
            let rotated = match angle {
                180 => image.rotate180(),
                -90 => image.rotate270(),
                90 => image.rotate90(),
                _ => image.clone(),
            };
            let attempt = self.read_image(&rotated);
            tracing::debug!(source = ?frame.source, rotation_clockwise_degrees = angle,
                elapsed_ms = started.elapsed().as_millis(), "MRZ orientation attempt completed");
            match attempt {
                Ok(mut reading) => {
                    reading.strip =
                        original_strip(reading.strip, image.width(), image.height(), angle);
                    if verified(&reading) {
                        tracing::debug!(
                            rotation_clockwise_degrees = angle,
                            "MRZ orientation found"
                        );
                        return Ok((reading, angle));
                    }
                    // Preserve diagnostics, but do not let an unverified zone
                    // prevent trying the remaining orientations.
                    if fallback.as_ref().is_none_or(|(r, _): &(Reading, i16)| {
                        r.lines.is_none() && reading.lines.is_some()
                    }) {
                        fallback = Some((reading, angle));
                    }
                }
                Err(e) => last = Some(e),
            }
        }
        fallback.ok_or_else(|| last.unwrap_or_else(|| Error::Malformed("no document found".into())))
    }

    fn read_image(&self, image: &DynamicImage) -> Result<Reading> {
        let grey = image.to_luma8();
        let (w, h) = grey.dimensions();

        let document = passport_scanner_imaging::crop::bounds(grey.as_raw(), w, h, self.threshold)
            .filter(|r| r.plausible(w, h))
            .ok_or_else(|| Error::Malformed("no document found on the platen".to_owned()))?;

        let strip = locate::zone_strip(document);
        let cropped = image.view(strip.x, strip.y, strip.w, strip.h).to_image();

        // Keep the established fast path for an already readable MRZ. A failed
        // attempt may be caused by a small line slope or by clipped edge glyphs.
        let original = self.read_ocr_image(DynamicImage::ImageRgba8(cropped), strip, SCALE)?;
        if verified(&original) {
            return Ok(original);
        }
        let expanded = document.grown((document.w / 100).max(2), w, h);
        let retry_strip = locate::zone_strip_for_skew(expanded);
        let band = image::imageops::crop_imm(
            &grey,
            retry_strip.x,
            retry_strip.y,
            retry_strip.w,
            retry_strip.h,
        )
        .to_image();
        let narrow_strip = passport_scanner_core::Rect {
            x: expanded.x,
            w: expanded.w,
            ..locate::zone_strip(expanded)
        };
        let narrow_band = image::imageops::crop_imm(
            &grey,
            narrow_strip.x,
            narrow_strip.y,
            narrow_strip.w,
            narrow_strip.h,
        )
        .to_image();
        // The usual strip insets both sides; a card lying slightly off-centre
        // can put the last check digit inside that inset. When the first pass
        // plainly saw a zone, read the same band at full width before
        // concluding anything, with or without measurable skew.
        let original = if text::looks_like_zone(&original.text) {
            let full = image.view(narrow_strip.x, narrow_strip.y, narrow_strip.w, narrow_strip.h);
            let full = self.read_ocr_image(
                DynamicImage::ImageRgba8(full.to_image()),
                narrow_strip,
                SCALE,
            )?;
            if verified(&full) {
                return Ok(full);
            }
            if original.lines.is_none() && full.lines.is_some() { full } else { original }
        } else {
            original
        };
        let Some(estimate) = deskew::estimate(&band).or_else(|| deskew::estimate(&narrow_band))
        else {
            return Ok(original);
        };
        tracing::debug!(
            correction_degrees = estimate.correction_degrees,
            confidence = estimate.confidence,
            "Retrying MRZ with OCR-only deskew"
        );
        let mut best = original;
        // Native-size text avoids magnifying interpolation artifacts. The
        // wider vertical band is a final retry for lines displaced by skew.
        // Both attempts preserve the complete document width and require the
        // same structural and checksum checks as the original reading.
        for (strip, pixels) in [(narrow_strip, narrow_band), (retry_strip, band)] {
            let aligned = deskew::rotate_expanded(&pixels, estimate.correction_degrees);
            let corrected = self.read_ocr_image(DynamicImage::ImageLuma8(aligned), strip, 1)?;
            if verified(&corrected) {
                return Ok(corrected);
            }
            if best.lines.is_none() && corrected.lines.is_some() {
                best = corrected;
            }
        }
        Ok(best)
    }

    fn read_ocr_image(
        &self,
        image: DynamicImage,
        strip: passport_scanner_core::Rect,
        scale: u32,
    ) -> Result<Reading> {
        let started = std::time::Instant::now();

        // Prepare OCR input without discarding antialiased character strokes.
        // The captured/processed document image itself is left untouched.
        let scaled = image
            .resize(
                image.width().saturating_mul(scale).min(6000),
                image.height().saturating_mul(scale).min(2000),
                image::imageops::FilterType::Lanczos3,
            )
            .to_luma8();

        let preparation_ms = started.elapsed().as_millis();
        let ocr_started = std::time::Instant::now();
        let text = self.engine.read(&scaled)?;
        let ocr_ms = ocr_started.elapsed().as_millis();
        let lines = text::zone_lines(&text);
        tracing::debug!(preparation_ms, ocr_ms, "MRZ image processing completed");
        Ok(Reading { strip, text, lines })
    }
}

/// Whether two verified documents differ only in what the composite check
/// digit alone protects: optional data, and the zone text that carries it.
fn only_composite_covered_differ(a: &Document, b: &Document) -> bool {
    use passport_scanner_core::FieldKind;
    let keys: std::collections::BTreeSet<_> = a.fields.keys().chain(b.fields.keys()).collect();
    keys.into_iter()
        .filter(|k| !matches!(k, FieldKind::OptionalData | FieldKind::MrzLine))
        .all(|k| a.fields.get(k) == b.fields.get(k))
}

/// Non-filler characters in a TD1 grid's optional-data fields.
fn optional_letters(lines: &[String]) -> usize {
    if lines.len() != 3 || lines.iter().any(|l| l.len() != 30) {
        return usize::MAX;
    }
    lines[0][15..].bytes().chain(lines[1][18..29].bytes()).filter(|&c| c != b'<').count()
}

fn verified(reading: &Reading) -> bool {
    reading.lines.as_ref().is_some_and(|lines| {
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        passport_scanner_mrz::parse(&refs).is_ok_and(|document| document.checksums_valid())
    })
}

fn original_strip(
    r: passport_scanner_core::Rect,
    w: u32,
    h: u32,
    angle: i16,
) -> passport_scanner_core::Rect {
    use passport_scanner_core::Rect;
    match angle {
        180 => Rect { x: w - r.x - r.w, y: h - r.y - r.h, ..r },
        -90 => Rect { x: w - r.y - r.h, y: r.x, w: r.h, h: r.w },
        90 => Rect { x: r.y, y: h - r.x - r.w, w: r.h, h: r.w },
        _ => r,
    }
}

impl<E: Engine> DocumentRecognizer for MrzReader<E> {
    fn name(&self) -> &str {
        "machine-readable zone"
    }

    fn recognize(&self, frames: &[Frame]) -> Result<Document> {
        self.recognize_oriented(frames).map(|result| result.document)
    }
}

impl<E: Engine> MrzReader<E> {
    /// Read the MRZ and determine how the captured side should be displayed.
    /// Verified MRZ rotations take precedence. A side without an MRZ may use
    /// the engine's general text orientation, which never supplies personal fields.
    pub fn recognize_oriented(&self, frames: &[Frame]) -> Result<Recognition> {
        // Infrared renders the zone's carbon ink at high contrast and drops the
        // printed background, so it is tried first where both exist.
        let ordered = {
            let mut v: Vec<&Frame> = frames.iter().collect();
            v.sort_by_key(|f| u8::from(f.source != LightSource::Infrared));
            v
        };

        let mut last = None;
        let mut best: Option<Document> = None;
        let mut best_lines: Vec<String> = Vec::new();
        let mut best_rank = (false, 0);
        let mut ambiguous = false;
        let mut verified_rotations = Vec::new();
        // Bound concurrency to two sources. Preserve each source's complete
        // ordered orientation search and compare every verified result below.
        let readings: Vec<_> = if self.engine.parallel_sources() {
            ordered
                .chunks(2)
                .flat_map(|pair| {
                    std::thread::scope(|scope| {
                        let jobs: Vec<_> = pair
                            .iter()
                            .map(|frame| scope.spawn(|| self.read_frame_oriented(frame)))
                            .collect();
                        jobs.into_iter()
                            .map(|job| {
                                job.join().unwrap_or_else(|_| {
                                    Err(Error::Rejected("OCR worker panicked".into()))
                                })
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect()
        } else {
            ordered.iter().map(|frame| self.read_frame_oriented(frame)).collect()
        };
        for reading in readings {
            match reading {
                Ok((reading, angle)) => {
                    let Some(lines) = reading.lines else {
                        last = Some(Error::Malformed(
                            "no complete zone in the recognised text".into(),
                        ));
                        continue;
                    };
                    let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
                    match passport_scanner_mrz::parse(&borrowed) {
                        Ok(doc) => {
                            // The public parser keeps failed checks for diagnostics;
                            // OCR recognition must not publish those personal fields.
                            if !doc.checksums_valid() {
                                continue;
                            }
                            verified_rotations.push(angle);
                            let rank = (
                                doc.checksums_valid(),
                                doc.fields.values().filter(|f| f.checksum_ok == Some(true)).count(),
                            );
                            let canonical = text::canonical_fillers(&lines);
                            if best.is_none() || rank > best_rank {
                                best = Some(doc);
                                best_lines = lines;
                                best_rank = rank;
                                ambiguous = false;
                            } else if rank == best_rank
                                && best.as_ref().is_some_and(|b| b.fields != doc.fields)
                                && canonical == text::canonical_fillers(&best_lines)
                            {
                                // One source saw fillers where the other saw
                                // their look-alikes: the same zone, so keep the
                                // fillers that were actually observed.
                                let refs: Vec<&str> =
                                    canonical.iter().map(String::as_str).collect();
                                if canonical != best_lines
                                    && let Ok(merged) = passport_scanner_mrz::parse(&refs)
                                    && merged.checksums_valid()
                                {
                                    best = Some(merged);
                                    best_lines = canonical;
                                }
                            } else if rank == best_rank
                                && best
                                    .as_ref()
                                    .is_some_and(|b| only_composite_covered_differ(b, &doc))
                                && optional_letters(&canonical)
                                    != optional_letters(&text::canonical_fillers(&best_lines))
                            {
                                // Every individually checked field and the name
                                // agree; only what the single composite digit
                                // covers differs, so one source hit a checksum
                                // collision. Fillers read as letters is the error
                                // OCR makes on this zone, never the reverse, so the
                                // reading with fewer optional letters is the one.
                                if optional_letters(&canonical)
                                    < optional_letters(&text::canonical_fillers(&best_lines))
                                {
                                    best = Some(doc);
                                    best_lines = lines;
                                }
                            } else if rank == best_rank
                                && best.as_ref().is_some_and(|b| b.fields != doc.fields)
                            {
                                // In particular, names are not covered by checksums.
                                // Conflicting equally supported sources must not
                                // silently select an IR transcription by lamp order.
                                ambiguous = true;
                            }
                        }
                        Err(e) => last = Some(e),
                    }
                }
                Err(e) => last = Some(e),
            }
        }
        let rotation = verified_rotations
            .first()
            .copied()
            .filter(|first| verified_rotations.iter().all(|angle| angle == first));
        if let Some(mut document) = best {
            // German identity cards carry no optional data at all, so filler
            // look-alikes there are misreads even where the composite check,
            // blind to K, cannot say so.
            if text::optional_data_always_empty(&best_lines) {
                let canonical = text::canonical_fillers(&best_lines);
                let refs: Vec<&str> = canonical.iter().map(String::as_str).collect();
                if let Ok(cleaned) = passport_scanner_mrz::parse(&refs)
                    && cleaned.checksums_valid()
                {
                    document = cleaned;
                }
            }
            return Ok(Recognition {
                document: if ambiguous { Document::default() } else { document },
                rotation,
            });
        }
        // Nothing found is not an error: a document may simply have no zone.
        match last {
            Some(Error::Malformed(_)) | None => {}
            Some(e) => return Err(e),
        }
        // Visible ink carries the readable text on fronts. Do not let IR
        // background loss make a confident visible orientation ambiguous.
        let orientation_frame =
            frames.iter().find(|f| f.source == LightSource::White).or_else(|| frames.first());
        let rotation = orientation_frame.and_then(|frame| {
            let image = decode(frame).ok()?;
            self.engine.orientation(&image.to_luma8()).ok().flatten()
        });
        Ok(Recognition { document: Document::default(), rotation })
    }
}

fn decode(frame: &Frame) -> Result<DynamicImage> {
    match frame.format {
        PixelFormat::Jpeg => {
            let mut decoder = ImageReader::new(Cursor::new(&frame.data))
                .with_guessed_format()
                .map_err(Error::from)?
                .into_decoder()
                .map_err(|e| Error::Malformed(format!("the frame is not a readable JPEG: {e}")))?;
            let orientation = decoder.orientation().map_err(Error::backend)?;
            let mut image = DynamicImage::from_decoder(decoder).map_err(Error::backend)?;
            image.apply_orientation(orientation);
            Ok(image)
        }
        PixelFormat::Gray8 => {
            image::GrayImage::from_raw(frame.width, frame.height, frame.data.clone())
                .map(DynamicImage::ImageLuma8)
                .ok_or_else(|| Error::Malformed("grey frame is shorter than its size".to_owned()))
        }
        PixelFormat::Rgb8 => {
            image::RgbImage::from_raw(frame.width, frame.height, frame.data.clone())
                .map(DynamicImage::ImageRgb8)
                .ok_or_else(|| Error::Malformed("rgb frame is shorter than its size".to_owned()))
        }
        // PixelFormat is non-exhaustive: a format added later lands here rather
        // than being silently misread as one of the above.
        other => Err(Error::unsupported(format!("{other:?} frames are not decoded here"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    const SPECIMEN: &str = "I<UTOD231458907<<<<<<<<<<<<<<<\n7408122F1204159UTO<<<<<<<<<<<6\nERIKSSON<<ANNA<MARIA<<<<<<<<<<";

    fn frame(image: image::GrayImage, source: LightSource) -> Frame {
        Frame {
            width: image.width(),
            height: image.height(),
            format: PixelFormat::Gray8,
            source,
            data: image.into_raw(),
        }
    }

    #[derive(Debug)]
    struct MarkerEngine(Arc<AtomicUsize>);
    impl Engine for MarkerEngine {
        fn read(&self, image: &image::GrayImage) -> Result<String> {
            self.0.fetch_add(1, Ordering::Relaxed);
            // A synthetic asymmetric mark at the lower-left of an upright card.
            // Recognize it only in a landscape strip and at its expected x.
            let black: Vec<_> = image.enumerate_pixels().filter(|(_, _, p)| p.0[0] < 128).collect();
            let x = black.iter().map(|(x, _, _)| f64::from(*x)).sum::<f64>()
                / black.len().max(1) as f64
                / f64::from(image.width());
            Ok(
                if image.width() > image.height() * 3
                    && !black.is_empty()
                    && (0.2..0.4).contains(&x)
                {
                    SPECIMEN.to_owned()
                } else {
                    String::new()
                },
            )
        }
    }

    #[test]
    fn actual_pixel_rotations_are_tried_in_requested_order_without_mutating_frame() {
        let upright = image::GrayImage::from_fn(120, 80, |x, y| {
            image::Luma([if (30..45).contains(&x) && (60..70).contains(&y) { 0 } else { 230 }])
        });
        for (pixels, expected_angle, expected_calls) in [
            (upright.clone(), 0, 1),
            (image::imageops::rotate180(&upright), 180, 2),
            (image::imageops::rotate90(&upright), -90, 3),
            (image::imageops::rotate270(&upright), 90, 4),
        ] {
            let calls = Arc::new(AtomicUsize::new(0));
            let reader = MrzReader::new(MarkerEngine(calls.clone()));
            let input = frame(pixels, LightSource::White);
            let original = input.data.clone();
            let (reading, angle) = reader.read_frame_oriented(&input).unwrap();
            assert_eq!(angle, expected_angle);
            assert_eq!(calls.load(Ordering::Relaxed), expected_calls);
            assert!(reading.lines.is_some());
            assert_eq!(input.data, original);
            assert!(reading.strip.x + reading.strip.w <= input.width);
            assert!(reading.strip.y + reading.strip.h <= input.height);
        }
    }

    #[test]
    fn verified_source_rotations_are_preserved_for_display() {
        let upright = image::GrayImage::from_fn(120, 80, |x, y| {
            image::Luma([if (30..45).contains(&x) && (60..70).contains(&y) { 0 } else { 230 }])
        });
        for (pixels, expected) in [
            (upright.clone(), 0),
            (image::imageops::rotate180(&upright), 180),
            (image::imageops::rotate90(&upright), -90),
            (image::imageops::rotate270(&upright), 90),
        ] {
            let reader = MrzReader::new(MarkerEngine(Arc::new(AtomicUsize::new(0))));
            let result = reader
                .recognize_oriented(&[
                    frame(pixels.clone(), LightSource::White),
                    frame(pixels, LightSource::Infrared),
                ])
                .unwrap();
            assert_eq!(result.rotation, Some(expected));
            assert!(result.document.checksums_valid());
        }
        let reader = MrzReader::new(MarkerEngine(Arc::new(AtomicUsize::new(0))));
        let conflict = reader
            .recognize_oriented(&[
                frame(upright.clone(), LightSource::White),
                frame(image::imageops::rotate180(&upright), LightSource::Infrared),
            ])
            .unwrap();
        assert_eq!(
            conflict.rotation, None,
            "contradictory source orientations must not be guessed"
        );
    }

    #[test]
    fn text_orientation_cannot_fill_fields_or_override_a_verified_mrz() {
        #[derive(Debug)]
        struct OrientedText {
            mrz: bool,
            orientation_calls: AtomicUsize,
        }
        impl Engine for OrientedText {
            fn read(&self, _: &image::GrayImage) -> Result<String> {
                Ok(if self.mrz { SPECIMEN.into() } else { String::new() })
            }
            fn orientation(&self, _: &image::GrayImage) -> Result<Option<i16>> {
                self.orientation_calls.fetch_add(1, Ordering::Relaxed);
                Ok(Some(180))
            }
        }
        for mrz in [false, true] {
            let reader =
                MrzReader::new(OrientedText { mrz, orientation_calls: AtomicUsize::new(0) });
            let result = reader
                .recognize_oriented(&[frame(
                    image::GrayImage::from_pixel(120, 80, image::Luma([230])),
                    LightSource::White,
                )])
                .unwrap();
            assert_eq!(result.rotation, Some(if mrz { 0 } else { 180 }));
            assert_eq!(result.document.fields.is_empty(), !mrz);
            assert_eq!(reader.engine.orientation_calls.load(Ordering::Relaxed), usize::from(!mrz));
        }
    }

    #[derive(Debug)]
    struct QueueEngine(Mutex<std::collections::VecDeque<String>>);
    impl Engine for QueueEngine {
        fn read(&self, _: &image::GrayImage) -> Result<String> {
            Ok(self.0.lock().unwrap().pop_front().unwrap_or_default())
        }
    }
    fn recognize_texts(texts: Vec<String>) -> Document {
        let reader = MrzReader::new(QueueEngine(Mutex::new(texts.into())));
        let pixels = image::GrayImage::from_pixel(120, 80, image::Luma([230]));
        reader
            .recognize(&[
                frame(pixels.clone(), LightSource::White),
                frame(pixels, LightSource::Infrared),
            ])
            .unwrap()
    }

    #[test]
    fn a_verified_visible_zone_beats_an_unverified_infrared_zone() {
        let bad = SPECIMEN.replace("D231458907", "D231458908");
        let doc = recognize_texts(vec![
            bad,
            String::new(),
            String::new(),
            String::new(),
            SPECIMEN.into(),
        ]);
        assert!(doc.checksums_valid());
        assert_eq!(
            doc.get(passport_scanner_core::FieldKind::DocumentNumber).unwrap().value,
            "D23145890"
        );
    }

    #[test]
    fn sources_differing_only_in_misread_fillers_agree_on_the_fillers() {
        // Both verify (K weighs like a filler), but put the K elsewhere: the
        // other source saw `<` there, so neither K is data.
        let white = SPECIMEN.replacen("907<<<<<<<<<<<<<<<", "907<<<<<<<<<K<<<<<", 1);
        let infrared = SPECIMEN.replacen("907<<<<<<<<<<<<<<<", "907<<<<<K<<<<<<<<<", 1);
        let doc = recognize_texts(vec![infrared, white]);
        assert!(doc.checksums_valid());
        assert_eq!(doc.fields, recognize_texts(vec![SPECIMEN.into(), SPECIMEN.into()]).fields);
        // The same K in both sources may be real optional data and is kept.
        let both = SPECIMEN.replacen("907<<<<<<<<<<<<<<<", "907<<<<<K<<<<<<<<<", 1);
        let kept = recognize_texts(vec![both.clone(), both]);
        assert!(kept.checksums_valid());
        assert_ne!(kept.fields, recognize_texts(vec![SPECIMEN.into(), SPECIMEN.into()]).fields);
    }

    #[test]
    fn a_composite_collision_loses_to_the_reading_with_fillers() {
        // A filler read as G at position 16 and the composite digit misread
        // as a matching wrong value: every check passes, yet it is wrong.
        let mut collided: Vec<String> = SPECIMEN.lines().map(str::to_owned).collect();
        collided[0].replace_range(15..16, "G");
        let composite = format!(
            "{}{}{}{}",
            &collided[0][5..],
            &collided[1][..7],
            &collided[1][8..15],
            &collided[1][18..29]
        );
        let cd = passport_scanner_mrz::check::check_digit(&composite).unwrap().to_string();
        collided[1].replace_range(29..30, &cd);
        let expected = recognize_texts(vec![SPECIMEN.into(), SPECIMEN.into()]).fields;
        for order in
            [[collided.join("\n"), SPECIMEN.into()], [SPECIMEN.into(), collided.join("\n")]]
        {
            let doc = recognize_texts(order.to_vec());
            assert!(doc.checksums_valid());
            assert_eq!(doc.fields, expected);
        }
        // Differing names stay ambiguous: see the test below.
    }

    #[test]
    fn german_identity_cards_never_show_misread_optional_data() {
        // Fictitious German TD1 zone (specimen data re-issued by "D").
        let mut rows: Vec<String> = SPECIMEN.lines().map(str::to_owned).collect();
        rows[0].replace_range(2..5, "D<<");
        rows[1].replace_range(15..18, "D<<");
        let composite =
            format!("{}{}{}{}", &rows[0][5..], &rows[1][..7], &rows[1][8..15], &rows[1][18..29]);
        let cd = passport_scanner_mrz::check::check_digit(&composite).unwrap().to_string();
        rows[1].replace_range(29..30, &cd);
        let clean = rows.join("\n");
        let noisy = clean.replacen("907<<<<<<<<<<<<<<<", "907<<<<<K<<K<<<<<", 1);
        let doc = recognize_texts(vec![noisy.clone(), noisy]);
        assert!(doc.checksums_valid());
        assert_eq!(doc.fields, recognize_texts(vec![clean.clone(), clean]).fields);
        assert!(
            doc.get(passport_scanner_core::FieldKind::OptionalData)
                .is_none_or(|f| f.value.trim().is_empty())
        );
    }

    #[test]
    fn conflicting_unchecksummed_names_are_not_silently_selected_by_source() {
        let doc = recognize_texts(vec![SPECIMEN.into(), SPECIMEN.replace("ANNA", "ANNE")]);
        assert!(doc.fields.is_empty());
    }

    #[test]
    fn a_front_with_no_zone_produces_no_personal_fields() {
        let doc = recognize_texts(vec!["ordinary front text".into(); 8]);
        assert!(doc.fields.is_empty());
    }

    #[derive(Debug)]
    struct ConcurrentEngine {
        started: (Mutex<usize>, std::sync::Condvar),
        conflict: bool,
    }
    impl Engine for ConcurrentEngine {
        fn parallel_sources(&self) -> bool {
            true
        }
        fn read(&self, image: &image::GrayImage) -> Result<String> {
            let (lock, ready) = &self.started;
            let mut count = lock.lock().unwrap();
            *count += 1;
            ready.notify_all();
            let (_guard, timeout) = ready
                .wait_timeout_while(count, std::time::Duration::from_secs(2), |n| *n < 2)
                .unwrap();
            assert!(!timeout.timed_out(), "both sources must run concurrently");
            Ok(if self.conflict && image.get_pixel(0, 0).0[0] < 200 {
                SPECIMEN.replace("ANNA", "ANNE")
            } else {
                SPECIMEN.into()
            })
        }
    }

    #[test]
    fn parallel_sources_still_compare_unchecksummed_names() {
        for conflict in [false, true] {
            let reader = MrzReader::new(ConcurrentEngine {
                started: (Mutex::new(0), std::sync::Condvar::new()),
                conflict,
            });
            let doc = reader
                .recognize(&[
                    frame(
                        image::GrayImage::from_pixel(120, 80, image::Luma([230])),
                        LightSource::White,
                    ),
                    frame(
                        image::GrayImage::from_pixel(120, 80, image::Luma([180])),
                        LightSource::Infrared,
                    ),
                ])
                .unwrap();
            assert_eq!(doc.fields.is_empty(), conflict);
            if !conflict {
                assert!(doc.checksums_valid());
            }
        }
    }

    #[test]
    fn a_panicking_parallel_engine_returns_an_error() {
        #[derive(Debug)]
        struct Panics;
        impl Engine for Panics {
            fn parallel_sources(&self) -> bool {
                true
            }
            fn read(&self, _: &image::GrayImage) -> Result<String> {
                panic!("engine failed");
            }
        }
        let result = MrzReader::new(Panics).recognize(&[
            frame(image::GrayImage::from_pixel(120, 80, image::Luma([230])), LightSource::White),
            frame(image::GrayImage::from_pixel(120, 80, image::Luma([230])), LightSource::Infrared),
        ]);
        assert!(matches!(result, Err(Error::Rejected(_))));
    }
}
