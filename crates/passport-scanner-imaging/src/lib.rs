//! Factory-calibrated capture processing.
//!
//! The default path measures the document crop, applies the matching factory
//! shading to the full frame, crops, and sharpens. Calibrated colour and tone
//! stay independent of scene content. Lighting estimation and document tone
//! modules remain available for explicitly constructed alternative pipelines.

#![forbid(unsafe_code)]

pub mod balance;
pub mod crop;
pub mod flatfield;
pub mod illumination;
pub mod levels;
pub mod sharpen;
pub mod tone;

use image::{DynamicImage, ImageEncoder, ImageReader, RgbImage, codecs::jpeg::JpegEncoder};
use passport_scanner_core::{Error, Frame, ImageProcessor, LightSource, PixelFormat, Rect, Result};
use std::io::Cursor;

pub use flatfield::Shading;
pub use illumination::Illumination;
pub use levels::{BlackWhite, Cuts};
pub use tone::DocumentTone;

/// Quality for re-encoding a processed frame.
///
/// High enough that the stage does not become the limit on what can be read.
pub const JPEG_QUALITY: u8 = 92;

/// How much of each edge the tone stages leave out when measuring.
///
/// A tenth each side, so what is measured is the middle four fifths. By the
/// time those stages run the frame has been cut down to the document, but a
/// card lying slightly askew still leaves slivers of platen in the corners -
/// and a shadow point taken from those is taken from the platen, not from the
/// print. Measuring an inset box and applying the result to the whole frame
/// avoids that without having to find the card a second time.
pub const INSET: f32 = 0.1;

/// One step of the work, on an image that is already decoded.
///
/// This is the seam the tone stages compose on; [`ImageProcessor`] is the seam
/// the rest of the stack sees.
pub trait Stage {
    /// What to call this step when reporting what was done.
    fn name(&self) -> &str;
    /// Apply it in place.
    fn apply(&self, image: &mut RgbImage);
}

/// Visit the pixels of the inset box, or all of them if it would be empty.
pub(crate) fn for_inset(image: &RgbImage, inset: f32, mut visit: impl FnMut([u8; 3])) {
    let (w, h) = image.dimensions();
    let dx = (w as f32 * inset) as u32;
    let dy = (h as f32 * inset) as u32;
    let (x0, x1) = if w > dx * 2 { (dx, w - dx) } else { (0, w) };
    let (y0, y1) = if h > dy * 2 { (dy, h - dy) } else { (0, h) };
    for y in y0..y1 {
        for x in x0..x1 {
            visit(image.get_pixel(x, y).0);
        }
    }
}

/// Correct for a platen the lamp does not light evenly.
///
/// Goes before everything else, because everything else assumes an evenly lit
/// frame. See [`flatfield`] for why that is not a given.
#[derive(Debug, Clone)]
pub struct FlatField {
    /// The unit's own calibration.
    pub shading: Shading,
    /// What a corrected white is scaled to.
    pub target: f32,
}

impl FlatField {
    /// Correction for a capture under the visible lamp.
    #[must_use]
    pub fn visible(shading: Shading) -> Self {
        Self { shading, target: flatfield::TARGET_VISIBLE }
    }
}

impl Stage for FlatField {
    fn name(&self) -> &str {
        "flat field"
    }

    fn apply(&self, image: &mut RgbImage) {
        let (w, h) = image.dimensions();

        // The calibration is of the whole sensor, so it can only be applied to
        // a whole frame: anything cropped or scaled no longer lines up with
        // the coefficients.
        for y in 0..h {
            for x in 0..w {
                let pixel = image.get_pixel_mut(x, y);
                for (channel, value) in pixel.0.iter_mut().enumerate() {
                    match self.shading.sample(x, y, w, h, channel) {
                        // The gain is TARGET / white_reference, so this puts
                        // what the platen returns at TARGET and everything
                        // else in proportion to it.
                        Some(gain) => {
                            // SHD stores target / reference, without a dark
                            // reference. Subtracting a percentile of the scene
                            // here changes that model and can turn a white
                            // reference into black. A dark-frame correction
                            // would require measuring both references together.
                            *value = (f32::from(*value) * gain).round().clamp(0.0, 255.0) as u8;
                        }
                        // Never calibrated: outside what the lens covers.
                        // Black is both honest and what the crop wants.
                        None => *value = 0,
                    }
                }
            }
        }
    }
}

/// Render an infrared capture as intensity rather than false RGB colour.
///
/// The camera's colour filters respond differently to infrared. Their RGB
/// ratios do not represent visible document colours. Use one shared intensity
/// before contrast adjustment so neither balancing nor stretching channels
/// separately invents colour in the infrared output.
#[derive(Debug, Clone, Copy, Default)]
pub struct Monochrome;

impl Stage for Monochrome {
    fn name(&self) -> &str {
        "monochrome"
    }

    fn apply(&self, image: &mut RgbImage) {
        for pixel in image.pixels_mut() {
            *pixel = image::Rgb([luma(pixel.0); 3]);
        }
    }
}

/// Take the lamp's colour cast off a capture, by its white point.
#[derive(Debug, Clone, Default)]
pub struct WhiteBalance {
    /// Fraction of each edge to leave out when measuring. See [`INSET`].
    pub inset: f32,
}

impl WhiteBalance {
    /// Balance measured over the document rather than its surroundings.
    #[must_use]
    pub fn for_documents() -> Self {
        Self { inset: INSET }
    }
}

impl Stage for WhiteBalance {
    fn name(&self) -> &str {
        "white balance"
    }

    fn apply(&self, image: &mut RgbImage) {
        let mut channels = [[0u64; 256]; 3];
        for_inset(image, self.inset, |p| {
            for (histogram, value) in channels.iter_mut().zip(p) {
                histogram[value as usize] += 1;
            }
        });

        let mut whites = [0f32; 3];
        for (white, histogram) in whites.iter_mut().zip(&channels) {
            match balance::highlight(histogram, balance::HIGHLIGHT) {
                Some(value) => *white = f32::from(value),
                None => return,
            }
        }

        let Some(gains) = balance::gains(whites) else {
            return;
        };
        let tables = balance::tables(gains);

        for p in image.pixels_mut() {
            for (channel, table) in p.0.iter_mut().zip(&tables) {
                *channel = table[*channel as usize];
            }
        }
    }
}

/// Stretch a capture's tones to the full range.
#[derive(Debug, Clone, Default)]
pub struct AutoLevels {
    /// Where to cut the histogram.
    pub cuts: Cuts,
    /// Midtone shaping. Below one lifts faint marks; one leaves them be.
    pub gamma: f32,
    /// Fraction of each edge to leave out when measuring. See [`INSET`].
    pub inset: f32,
    /// Stretch each channel to its own ends rather than all three together.
    ///
    /// A document is dark marks on light paper, and both of those are meant to
    /// be neutral. Stretching the three channels together preserves whatever
    /// cast is already there; stretching each to its own ends removes it at
    /// both ends at once - which is the only thing that takes out a cast that
    /// sits in the shadows but not in the paper.
    ///
    /// That kind of cast is not hypothetical here. Stray light inside the
    /// housing adds a little of the lamp's own colour to everything, and being
    /// additive it counts for far more against a dark tone than a light one:
    /// measured on a corrected capture, the paper came back neutral to within
    /// three levels while the print was nineteen levels green.
    ///
    /// The cost is that a genuinely coloured document loses some of its
    /// colour. For an identity card, whose ground is white by design, that
    /// trade is worth making.
    pub per_channel: bool,
}

impl AutoLevels {
    /// Levels tuned for an identity document under its own lamp.
    #[must_use]
    pub fn for_documents() -> Self {
        Self { cuts: Cuts::default(), gamma: 0.85, inset: INSET, per_channel: true }
    }
}

impl Stage for AutoLevels {
    fn name(&self) -> &str {
        "auto levels"
    }

    fn apply(&self, image: &mut RgbImage) {
        if self.per_channel {
            let mut histograms = [[0u64; 256]; 3];
            for_inset(image, self.inset, |p| {
                for (histogram, value) in histograms.iter_mut().zip(p) {
                    histogram[value as usize] += 1;
                }
            });

            // A channel with nothing to stretch is left alone rather than
            // holding up the other two: forcing a curve onto a flat channel
            // only amplifies its noise.
            let tables: Vec<Option<[u8; 256]>> = histograms
                .iter()
                .map(|h| levels::black_white(h, self.cuts).map(|p| levels::table(p, self.gamma)))
                .collect();

            for pixel in image.pixels_mut() {
                for (value, table) in pixel.0.iter_mut().zip(&tables) {
                    if let Some(table) = table {
                        *value = table[*value as usize];
                    }
                }
            }
            return;
        }

        // One histogram over luminance, so the stretch cannot pull the
        // channels apart and tint the document.
        let mut histogram = [0u64; 256];
        for_inset(image, self.inset, |p| histogram[luma(p) as usize] += 1);

        let Some(points) = levels::black_white(&histogram, self.cuts) else {
            return;
        };
        let table = levels::table(points, self.gamma);

        for channel in image.as_mut() {
            *channel = table[*channel as usize];
        }
    }
}

/// Restore edge definition softened by the lamp and the codec.
#[derive(Debug, Clone)]
pub struct Sharpen {
    /// How much of the edge signal to add back.
    pub amount: f32,
    /// What size of detail to restore. See [`sharpen::unsharp_radius`].
    pub radius: usize,
}

impl Default for Sharpen {
    fn default() -> Self {
        // At 2592 pixels across an 86 mm card a stroke of print is three or
        // four pixels wide, so a one-pixel radius sharpens the grain inside
        // the stroke instead of the stroke - which is why output off this
        // scanner read as soft whatever the amount was. Two covers the
        // stroke; the amount is then enough to make small print crisp and
        // short of the halos that make a scan look processed.
        Self { amount: 1.0, radius: 2 }
    }
}

impl Stage for Sharpen {
    fn name(&self) -> &str {
        "sharpen"
    }

    fn apply(&self, image: &mut RgbImage) {
        *image = sharpen::unsharp_radius(image, self.amount, self.radius);
    }
}

/// Optional scene-derived lighting correction for explicitly constructed pipelines.
/// The default calibrated path uses the matching factory SHD instead.
#[derive(Debug, Clone, Copy, Default)]
pub struct Flatten;

impl Stage for Flatten {
    fn name(&self) -> &str {
        "flatten"
    }

    fn apply(&self, image: &mut RgbImage) {
        // No measurable background means nothing to measure against, and
        // leaving the frame alone is the right answer rather than a guess.
        if let Some(measured) = Illumination::measure(image) {
            measured.apply(image);
        }
    }
}

/// The treatment one capture gets, decoded and re-encoded once.
///
/// Not a free list of steps, because the order is not free. Two of them have
/// to straddle each other:
///
/// - The flat-field correction is of the whole sensor, so it can only be
///   applied to a whole frame - before any cropping.
/// - Where the document is has to be *measured* before that correction, not
///   after. The correction multiplies the dim end of the platen by up to five
///   and takes the contrast between platen and card with it; measured first,
///   on the frame as the sensor delivered it, that contrast is at its
///   greatest.
///
/// So the crop is found first, applied after, and the tone stages run last on
/// a frame that is by then the document and nothing else. That last part
/// matters as much as the rest: measured on a frame that is mostly platen, a
/// stretch is measuring the platen.
pub struct Enhance {
    /// Correct the platen's uneven lighting, where there is a calibration.
    pub flat_field: Option<FlatField>,
    /// Cut the frame down to the document.
    pub crop: bool,
    /// What to do with the document's tones once it is on its own.
    pub stages: Vec<Box<dyn Stage + Send + Sync>>,
}

impl std::fmt::Debug for Enhance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Enhance")
            .field("flat_field", &self.flat_field.is_some())
            .field("crop", &self.crop)
            .field("stages", &self.stages.iter().map(|s| s.name()).collect::<Vec<_>>())
            .finish()
    }
}

impl Enhance {
    /// The treatment an identity document gets.
    #[must_use]
    pub fn for_documents() -> Self {
        Self::for_documents_with(None, LightSource::White)
    }

    /// The same, with the unit's flat-field calibration where there is one.
    ///
    /// **The calibration replaces the white balance.** Each coefficient is
    /// `TARGET / white_reference` *per channel*, so applying it has already
    /// measured and set the white - three separate channels, corrected
    /// separately. Measuring it again afterwards is measuring an image that
    /// has already been measured, and the second answer is worse than the
    /// first because it has less to go on.
    ///
    /// Calibrated captures retain their measured tones. A scene-dependent
    /// stretch can erase the pale security print and independently stretching
    /// RGB channels changes the document's colours. The target of 240 also
    /// leaves headroom for highlights instead of forcing paper to clipped white.
    ///
    /// It is applied to a visible-light capture only, because that is what the
    /// file describes. The shipped calibration program writes one file per
    /// lamp and names it accordingly, and on the reference unit it has written
    /// none for infrared. Using the visible one there was tried: it takes the
    /// bottom off the frame, machine-readable zone and all, and flattens what
    /// contrast the engraving has. Infrared is the one source that does not
    /// need it anyway - the engraving absorbs it so strongly that uneven
    /// lighting barely shows.
    #[must_use]
    pub fn for_documents_with(shading: Option<Shading>, source: LightSource) -> Self {
        let flat_field = match (shading, source) {
            (Some(shading), LightSource::White) => Some(FlatField::visible(shading)),
            _ => None,
        };
        let mut stages: Vec<Box<dyn Stage + Send + Sync>> = Vec::with_capacity(3);
        if flat_field.is_none() {
            if source == LightSource::Infrared {
                stages.push(Box::new(Monochrome));
            } else {
                stages.push(Box::new(WhiteBalance::for_documents()));
            }
            // A single intensity curve preserves channel relationships. IR is
            // already monochrome, and must remain so through levels/sharpen.
            let mut levels = AutoLevels::for_documents();
            levels.per_channel = false;
            stages.push(Box::new(levels));
        }
        // Preserve the sharpening used by the validated factory-profile captures.
        stages.push(Box::new(Sharpen { amount: 0.8, radius: 1 }));
        Self { flat_field, crop: true, stages }
    }

    /// A treatment that does nothing, for a caller that wants the original.
    #[must_use]
    pub fn none() -> Self {
        Self { flat_field: None, crop: false, stages: Vec::new() }
    }

    /// Whether this would change a frame at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.flat_field.is_none() && !self.crop && self.stages.is_empty()
    }

    /// Run it over one decoded image.
    pub fn run(&self, image: &mut RgbImage) {
        self.run_observed(image, |_, _| {});
    }

    /// Run the same pipeline while observing each completed step.
    /// Diagnostic tools use this instead of rebuilding an approximate pipeline.
    pub fn run_observed(&self, image: &mut RgbImage, mut observe: impl FnMut(&str, &RgbImage)) {
        // Measured before the correction, applied after it. See this type's
        // own documentation for why that is not the obvious order.
        let region = self.crop.then(|| document_region(image));

        if let Some(flat_field) = &self.flat_field {
            flat_field.apply(image);
            observe("flat field", image);
        }
        if let Some(region) = region {
            let (w, h) = image.dimensions();
            if region != Rect::whole(w, h) {
                *image = image::imageops::crop_imm(image, region.x, region.y, region.w, region.h)
                    .to_image();
            }
            observe("crop", image);
        }
        for stage in &self.stages {
            stage.apply(image);
            observe(stage.name(), image);
        }
    }
}

/// Where the document is in a frame the sensor has just delivered.
///
/// Falls back to the whole frame when there is no dark surround to measure
/// against - a document filling the platen is a perfectly good scan.
fn document_region(image: &RgbImage) -> Rect {
    let (w, h) = image.dimensions();
    let mut histogram = [0u64; 256];
    for p in image.pixels() {
        histogram[luma(p.0) as usize] += 1;
    }
    let Some(threshold) = levels::platen(&histogram) else {
        return Rect::whole(w, h);
    };
    let grey: Vec<u8> = image.pixels().map(|p| luma(p.0)).collect();
    crop::region(&grey, w, h, threshold)
}

impl ImageProcessor for Enhance {
    fn name(&self) -> &str {
        "enhance"
    }

    fn process(&self, frame: &mut Frame) -> Result<()> {
        if self.is_empty() {
            return Ok(());
        }
        let mut image = decode(frame)?.to_rgb8();
        self.run(&mut image);
        encode(frame, &image)
    }
}

/// Run one stage on its own as a frame processor.
fn process_one<S: Stage>(stage: &S, frame: &mut Frame) -> Result<()> {
    let mut image = decode(frame)?.to_rgb8();
    stage.apply(&mut image);
    encode(frame, &image)
}

impl ImageProcessor for FlatField {
    fn name(&self) -> &str {
        Stage::name(self)
    }
    fn process(&self, frame: &mut Frame) -> Result<()> {
        process_one(self, frame)
    }
}

impl ImageProcessor for WhiteBalance {
    fn name(&self) -> &str {
        Stage::name(self)
    }
    fn process(&self, frame: &mut Frame) -> Result<()> {
        process_one(self, frame)
    }
}

impl ImageProcessor for AutoLevels {
    fn name(&self) -> &str {
        Stage::name(self)
    }
    fn process(&self, frame: &mut Frame) -> Result<()> {
        process_one(self, frame)
    }
}

impl ImageProcessor for Sharpen {
    fn name(&self) -> &str {
        Stage::name(self)
    }
    fn process(&self, frame: &mut Frame) -> Result<()> {
        process_one(self, frame)
    }
}

/// Rec. 601 luminance, which is what the eye reads text by.
#[must_use]
pub fn luma(rgb: [u8; 3]) -> u8 {
    let [r, g, b] = rgb.map(f32::from);
    (0.299 * r + 0.587 * g + 0.114 * b).round().clamp(0.0, 255.0) as u8
}

fn decode(frame: &Frame) -> Result<DynamicImage> {
    match frame.format {
        PixelFormat::Jpeg => ImageReader::new(Cursor::new(&frame.data))
            .with_guessed_format()
            .map_err(Error::from)?
            .decode()
            .map_err(|e| Error::Malformed(format!("the frame is not a readable JPEG: {e}"))),
        PixelFormat::Rgb8 => RgbImage::from_raw(frame.width, frame.height, frame.data.clone())
            .map(DynamicImage::ImageRgb8)
            .ok_or_else(|| Error::Malformed("rgb frame is shorter than its size".to_owned())),
        PixelFormat::Gray8 => {
            image::GrayImage::from_raw(frame.width, frame.height, frame.data.clone())
                .map(DynamicImage::ImageLuma8)
                .ok_or_else(|| Error::Malformed("grey frame is shorter than its size".to_owned()))
        }
        other => Err(Error::unsupported(format!("{other:?} frames are not processed here"))),
    }
}

fn encode(frame: &mut Frame, rgb: &RgbImage) -> Result<()> {
    let mut out = Vec::with_capacity(frame.data.len());
    JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
        .write_image(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)
        .map_err(|e| Error::Malformed(format!("could not re-encode the frame: {e}")))?;

    frame.width = rgb.width();
    frame.height = rgb.height();
    frame.format = PixelFormat::Jpeg;
    frame.data = out;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luminance_weights_green_most() {
        assert_eq!(luma([0, 0, 0]), 0);
        assert_eq!(luma([255, 255, 255]), 255);
        assert!(luma([0, 255, 0]) > luma([255, 0, 0]));
        assert!(luma([255, 0, 0]) > luma([0, 0, 255]));
    }

    #[test]
    fn document_levels_lift_the_midtones() {
        let l = AutoLevels::for_documents();
        assert!(l.gamma < 1.0, "faint engraving needs the midtones brought up");
    }

    fn frame_of(image: RgbImage) -> Frame {
        let mut frame = Frame {
            width: image.width(),
            height: image.height(),
            format: PixelFormat::Rgb8,
            source: LightSource::White,
            data: image.into_raw(),
        };
        // Put it through the codec, the way a real capture arrives.
        let rgb = decode(&frame).unwrap().to_rgb8();
        encode(&mut frame, &rgb).unwrap();
        frame
    }

    /// A card as the sensor delivers it: tones crowded into the top of the
    /// range, with the print only a little below the background.
    fn flat_card() -> Frame {
        frame_of(RgbImage::from_fn(64, 48, |x, y| {
            let v = if (10..54).contains(&x) && y % 8 < 3 { 182u8 } else { 205 };
            image::Rgb([v, v, v])
        }))
    }

    /// The same card lying on a dark platen, which is what a scan really is.
    fn card_on_platen() -> Frame {
        frame_of(RgbImage::from_fn(200, 150, |x, y| {
            if !(40..160).contains(&x) || !(30..120).contains(&y) {
                return image::Rgb([6, 7, 6]);
            }
            let v = if y % 8 < 3 { 182u8 } else { 205 };
            image::Rgb([v, v, v])
        }))
    }

    fn spread(frame: &Frame) -> u8 {
        let image = decode(frame).unwrap().to_rgb8();
        let (mut low, mut high) = (255u8, 0u8);
        for p in image.pixels() {
            let l = luma(p.0);
            low = low.min(l);
            high = high.max(l);
        }
        high - low
    }

    #[test]
    fn enhancing_a_flat_card_opens_the_tones_up() {
        let before = flat_card();
        let mut after = before.clone();
        Enhance::for_documents().process(&mut after).unwrap();
        assert!(
            u32::from(spread(&after)) > u32::from(spread(&before)) * 3,
            "a narrow card should come out far wider, got {} from {}",
            spread(&after),
            spread(&before)
        );
    }

    #[test]
    fn a_card_on_a_platen_is_cut_down_to_the_card() {
        let mut frame = card_on_platen();
        Enhance::for_documents().process(&mut frame).unwrap();
        assert!(frame.width < 160 && frame.width >= 110, "came out {} wide", frame.width);
        assert!(frame.height < 125 && frame.height >= 80, "came out {} high", frame.height);
    }

    /// Tones within one part of a frame, which is the only way to compare a
    /// cropped result with an uncropped one: measured over the whole of the
    /// latter, the black platen supplies the full range by itself and says
    /// nothing at all about the card.
    fn spread_within(frame: &Frame, area: Rect) -> u8 {
        let image = decode(frame).unwrap().to_rgb8();
        let (mut low, mut high) = (255u8, 0u8);
        for y in area.y..(area.y + area.h).min(image.height()) {
            for x in area.x..(area.x + area.w).min(image.width()) {
                let l = luma(image.get_pixel(x, y).0);
                low = low.min(l);
                high = high.max(l);
            }
        }
        high - low
    }

    #[test]
    fn the_stretch_is_measured_on_the_card_not_on_the_platen() {
        // The whole point of cropping before the tone stages. Measured over
        // the full frame the platen pins the shadow point at the bottom and
        // the card, the only part anyone wants to read, is barely touched.
        let mut cropped = card_on_platen();
        Enhance::for_documents().process(&mut cropped).unwrap();
        let on_card = spread_within(&cropped, Rect { x: 10, y: 10, w: 100, h: 70 });

        let mut uncropped = card_on_platen();
        Enhance { crop: false, ..Enhance::for_documents() }.process(&mut uncropped).unwrap();
        // The card sits here in the uncropped frame; see card_on_platen.
        let on_platen = spread_within(&uncropped, Rect { x: 50, y: 40, w: 100, h: 70 });

        assert!(
            u32::from(on_card) > u32::from(on_platen) + 60,
            "cropped {on_card} should open the card up far more than uncropped {on_platen}"
        );
    }

    #[test]
    fn enhancing_takes_the_lamps_cast_off() {
        // A card under the visible lamp: green ahead of red and blue, on a
        // platen the balance must not average in.
        let mut frame = frame_of(RgbImage::from_fn(200, 150, |x, y| {
            if !(40..160).contains(&x) || !(30..120).contains(&y) {
                return image::Rgb([3, 4, 3]);
            }
            let v = if y % 8 < 3 { 170u8 } else { 200 };
            image::Rgb([v - 40, v, v - 25])
        }));

        let cast = |f: &Frame| {
            let img = decode(f).unwrap().to_rgb8();
            let mut sums = [0u64; 3];
            let mut n = 0u64;
            for p in img.pixels() {
                if luma(p.0) <= 40 {
                    continue;
                }
                n += 1;
                for (s, c) in sums.iter_mut().zip(p.0) {
                    *s += u64::from(c);
                }
            }
            let m = sums.map(|s| s as f64 / n as f64);
            m.iter().copied().fold(f64::MIN, f64::max) - m.iter().copied().fold(f64::MAX, f64::min)
        };

        let before = cast(&frame);
        Enhance::for_documents().process(&mut frame).unwrap();
        let after = cast(&frame);
        assert!(after < before / 2.0, "the cast went from {before:.1} to {after:.1}");
    }

    #[test]
    fn a_frame_with_no_platen_to_find_keeps_its_size() {
        let mut frame = flat_card();
        Enhance::for_documents().process(&mut frame).unwrap();
        assert_eq!((frame.width, frame.height), (64, 48));
        assert_eq!(frame.format, PixelFormat::Jpeg);
    }

    #[test]
    fn an_empty_treatment_leaves_a_frame_byte_for_byte() {
        let frame = flat_card();
        let mut same = frame.clone();
        Enhance::none().process(&mut same).unwrap();
        assert_eq!(same.data, frame.data, "nothing to do should mean no re-encode");
    }

    #[test]
    fn a_calibration_replaces_scene_dependent_colour_and_tone_changes() {
        let bytes = vec![1u8; (flatfield::TRAILER + flatfield::GRID_W * flatfield::GRID_H * 3) * 4];
        let shading = Shading::parse(&bytes).unwrap();

        let calibrated = Enhance::for_documents_with(Some(shading), LightSource::White);
        let names: Vec<&str> = calibrated.stages.iter().map(|s| s.name()).collect();
        assert_eq!(
            names,
            ["sharpen"],
            "the white is already set per channel, so nothing should measure it again"
        );

        let uncalibrated = Enhance::for_documents_with(None, LightSource::White);
        let names: Vec<&str> = uncalibrated.stages.iter().map(|s| s.name()).collect();
        assert_eq!(names, ["white balance", "auto levels", "sharpen"]);
    }

    #[test]
    fn infrared_is_monochrome_before_its_shared_contrast_curve() {
        let plain = Enhance::for_documents_with(None, LightSource::Infrared);
        let names: Vec<&str> = plain.stages.iter().map(|s| s.name()).collect();
        assert_eq!(names, ["monochrome", "auto levels", "sharpen"]);
        let mut image = RgbImage::from_fn(80, 60, |_, y| {
            image::Rgb(if y % 12 < 3 { [80, 35, 70] } else { [235, 130, 215] })
        });
        let pipeline = Enhance { crop: false, ..plain };
        pipeline.run(&mut image);
        assert!(image.pixels().all(|p| p[0] == p[1] && p[1] == p[2]));
        assert!(image.get_pixel(40, 6)[0] > image.get_pixel(40, 1)[0]);
    }

    #[test]
    fn infrared_is_left_without_a_flat_field_even_when_one_is_offered() {
        // There is no infrared calibration, and using the visible one there
        // was tried: it takes the bottom off the frame.
        let bytes = vec![0u8; (flatfield::TRAILER + flatfield::GRID_W * flatfield::GRID_H * 3) * 4];
        let shading = Shading::parse(&bytes).unwrap();
        let ir = Enhance::for_documents_with(Some(shading.clone()), LightSource::Infrared);
        assert!(ir.flat_field.is_none());
        let white = Enhance::for_documents_with(Some(shading), LightSource::White);
        assert!(white.flat_field.is_some());
    }

    #[test]
    fn applying_a_measured_calibration_returns_its_white_target() {
        let mut reference = RgbImage::from_pixel(64, 48, image::Rgb([180, 210, 195]));
        let shading = Shading::from_reference(&reference, flatfield::TARGET_VISIBLE);
        FlatField::visible(shading).apply(&mut reference);
        assert!(reference.pixels().all(|p| p.0 == [240; 3]));
    }

    #[test]
    fn calibrated_colour_does_not_depend_on_the_surrounding_document() {
        let reference = RgbImage::from_pixel(64, 48, image::Rgb([240; 3]));
        let shading = Shading::from_reference(&reference, flatfield::TARGET_VISIBLE);
        let enhancement = Enhance {
            crop: false,
            ..Enhance::for_documents_with(Some(shading), LightSource::White)
        };
        for background in [[225, 210, 235], [180, 220, 200]] {
            let mut document = RgbImage::from_fn(64, 48, |x, y| {
                image::Rgb(if (20..44).contains(&x) && (16..32).contains(&y) {
                    [190, 170, 205]
                } else {
                    background
                })
            });
            enhancement.run(&mut document);
            assert_eq!(document.get_pixel(32, 24).0, [190, 170, 205]);
            assert_eq!(document.get_pixel(8, 8).0, background);
        }
    }
    #[test]
    fn explicit_document_tone_keeps_a_neutral_document_neutral() {
        // Retain the optional DocumentTone regression without making it the default.
        let enhancement = Enhance {
            crop: false,
            flat_field: None,
            stages: vec![Box::new(DocumentTone::for_documents()), Box::new(Sharpen::default())],
        };
        for paper in [205u8, 170] {
            let print = paper - 60;
            let mut document = RgbImage::from_fn(200, 150, |_, y| {
                image::Rgb(if y % 12 < 2 { [print; 3] } else { [paper; 3] })
            });
            enhancement.run(&mut document);

            let out_paper = document.get_pixel(100, 5).0;
            let out_print = document.get_pixel(100, 0).0;
            let spread = |p: [u8; 3]| {
                i32::from(p.iter().copied().max().unwrap())
                    - i32::from(p.iter().copied().min().unwrap())
            };
            assert!(spread(out_paper) <= 3, "paper came out tinted: {out_paper:?}");
            assert!(spread(out_print) <= 6, "print came out tinted: {out_print:?}");
            assert!(out_paper[0] > out_print[0], "the print should stay below the paper");
            assert!(out_paper[0] >= 235, "paper from {paper} landed at {out_paper:?}");
        }
    }
}
