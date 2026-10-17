//! Small-angle correction of an OCR-only MRZ strip. Captured images are untouched.
//!
//! Sparse vertical-stroke contrast votes into candidate horizontal projections.
//! Solid platen borders contribute little; at least two text-like bands and
//! distributed horizontal evidence are required before changing an image.
use image::{GrayImage, Luma};

/// A measured correction, not the angle of the original text baseline.
#[derive(Debug, Clone, Copy)]
pub struct Estimate {
    /// Clockwise image rotation in degrees. Text descending to the right needs
    /// a negative correction. Bounded to Ãƒâ€šÃ‚Â±5 degrees.
    pub correction_degrees: f32,
    /// Relative projection improvement over zero skew, clamped to 0..1.
    /// This is a geometric confidence score, not a probability of correct OCR.
    pub confidence: f32,
}

/// Estimate a correction only when multiple text bands support it.
pub fn estimate(image: &GrayImage) -> Option<Estimate> {
    if image.width() < 80 || image.height() < 24 {
        return None;
    }
    let scale = (800.0 / image.width() as f32).min(300.0 / image.height() as f32).min(1.0);
    let sampled = image::imageops::resize(
        image,
        (image.width() as f32 * scale).round().max(1.) as u32,
        (image.height() as f32 * scale).round().max(1.) as u32,
        image::imageops::FilterType::Triangle,
    );
    let (width, height) = sampled.dimensions();
    if width < 80 || height < 24 {
        return None;
    }
    let mut points = Vec::new();
    let mut bins = [0usize; 8];
    let inset = (width / 50).max(3);
    for y in 2..height - 2 {
        for x in inset..width - inset {
            let center = sampled.get_pixel(x, y).0[0] as i16;
            let neighbor =
                sampled.get_pixel(x - 2, y).0[0].max(sampled.get_pixel(x + 2, y).0[0]) as i16;
            let contrast = neighbor - center;
            if neighbor >= 110 && contrast >= 40 && center <= 185 {
                points.push((x as f32 - width as f32 / 2., y as f32, contrast.min(100) as f32));
                bins[(x as usize * 8 / width as usize).min(7)] += 1;
            }
        }
    }
    if points.len() < 200 || bins.iter().filter(|&&count| count >= 20).count() < 5 {
        return None;
    }
    let margin = (width as f32 * 6f32.to_radians().tan()).ceil() as usize + 3;
    let mut histogram = vec![0f32; height as usize + margin * 2 + 2];
    let mut scores = [0f64; 49];
    for (index, score) in scores.iter_mut().enumerate() {
        let angle = (index as f32 - 24.) * 0.25;
        project(&points, angle, margin, &mut histogram);
        *score = histogram.iter().map(|&value| f64::from(value).powi(2)).sum();
    }
    let (best_index, &best) = scores.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))?;
    let baseline_degrees = (best_index as f32 - 24.) * 0.25;
    if baseline_degrees.abs() < 0.5 || baseline_degrees.abs() > 5.0 {
        return None;
    }
    let gain = best / scores[24].max(1.) - 1.;
    let distant = scores
        .iter()
        .enumerate()
        .filter(|(index, _)| index.abs_diff(best_index) >= 3)
        .map(|(_, score)| *score)
        .fold(0f64, f64::max);
    project(&points, baseline_degrees, margin, &mut histogram);
    let bands = text_bands(&histogram);
    let separation = best / distant.max(1.);
    // Near zero, the upright projection is already close to optimal: its
    // relative gain can stay below 4% despite a well-isolated angle peak.
    // Accept that smaller gain only with three text bands and much stronger
    // angular separation. This is geometry evidence, never OCR validation.
    let small_angle =
        baseline_degrees.abs() < 1.0 && gain >= 0.02 && separation >= 1.05 && bands >= 3;
    let ordinary = gain >= 0.04 && separation >= 1.012 && bands >= 2;
    if !small_angle && !ordinary {
        return None;
    }
    Some(Estimate { correction_degrees: -baseline_degrees, confidence: gain.clamp(0., 1.) as f32 })
}

fn project(points: &[(f32, f32, f32)], angle: f32, margin: usize, histogram: &mut [f32]) {
    histogram.fill(0.);
    let slope = angle.to_radians().tan();
    for &(x, y, weight) in points {
        let row = y - slope * x + margin as f32;
        let index = row.floor() as usize;
        let fraction = row - index as f32;
        if index + 1 < histogram.len() {
            histogram[index] += weight * (1. - fraction);
            histogram[index + 1] += weight * fraction;
        }
    }
}

fn text_bands(histogram: &[f32]) -> usize {
    let threshold = histogram.iter().copied().fold(0f32, f32::max) * 0.30;
    let mut bands = 0;
    let mut start = None;
    let mut last = 0;
    for (index, &value) in histogram.iter().enumerate() {
        if value > threshold {
            start.get_or_insert(index);
            last = index;
        } else if start.is_some() && index - last > 2 {
            let height = last - start.take().unwrap() + 1;
            if (3..=36).contains(&height) {
                bands += 1;
            }
        }
    }
    if let Some(start) = start {
        if (3..=36).contains(&(last - start + 1)) {
            bands += 1;
        }
    }
    bands
}

/// Return an expanded, padded OCR image and its correction, or abstain.
pub fn prepare(image: &GrayImage) -> Option<(GrayImage, Estimate)> {
    let estimate = estimate(image)?;
    Some((rotate_expanded(image, estimate.correction_degrees), estimate))
}

/// Rotate clockwise with bilinear sampling and a complete expanded canvas.
/// Two extra pixels surround the rotated image. Bright padding is chosen from
/// the input's 90th luminance percentile (at least 180), not black platen edges.
pub fn rotate_expanded(image: &GrayImage, clockwise_degrees: f32) -> GrayImage {
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 || !clockwise_degrees.is_finite() {
        return image.clone();
    }
    let radians = clockwise_degrees.to_radians();
    let (sin, cos) = radians.sin_cos();
    let out_width = (width as f32 * cos.abs() + height as f32 * sin.abs()).ceil() as u32 + 4;
    let out_height = (height as f32 * cos.abs() + width as f32 * sin.abs()).ceil() as u32 + 4;
    let mut counts = [0usize; 256];
    for pixel in image.as_raw() {
        counts[*pixel as usize] += 1;
    }
    let target = image.as_raw().len() * 9 / 10;
    let mut sum = 0;
    let background = counts
        .iter()
        .enumerate()
        .find_map(|(value, count)| {
            sum += count;
            (sum > target).then_some(value as u8)
        })
        .unwrap_or(255)
        .max(180);
    let source_x = (width - 1) as f32 / 2.;
    let source_y = (height - 1) as f32 / 2.;
    GrayImage::from_fn(out_width, out_height, |x, y| {
        let dx = x as f32 - (out_width - 1) as f32 / 2.;
        let dy = y as f32 - (out_height - 1) as f32 / 2.;
        let x = cos * dx + sin * dy + source_x;
        let y = -sin * dx + cos * dy + source_y;
        let left = x.floor() as i32;
        let top = y.floor() as i32;
        let fx = x - left as f32;
        let fy = y - top as f32;
        let sample = |x: i32, y: i32| -> f32 {
            if x >= 0 && y >= 0 && (x as u32) < width && (y as u32) < height {
                image.get_pixel(x as u32, y as u32).0[0] as f32
            } else {
                background as f32
            }
        };
        let value = sample(left, top) * (1. - fx) * (1. - fy)
            + sample(left + 1, top) * fx * (1. - fy)
            + sample(left, top + 1) * (1. - fx) * fy
            + sample(left + 1, top + 1) * fx * fy;
        Luma([value.round().clamp(0., 255.) as u8])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn text() -> GrayImage {
        let glyphs = [
            [31u8, 4, 4, 4, 4, 4, 31],
            [17, 25, 25, 21, 19, 19, 17],
            [14, 17, 17, 17, 17, 17, 14],
            [2, 4, 8, 16, 8, 4, 2],
            [14, 17, 17, 31, 17, 17, 17],
            [30, 17, 17, 30, 20, 18, 17],
        ];
        let mut image = GrayImage::from_pixel(760, 180, Luma([235]));
        for row in 0..3 {
            for character in 0..44 {
                let glyph = &glyphs[(character + row * 3) % glyphs.len()];
                for (y, bits) in glyph.iter().enumerate() {
                    for x in 0..5 {
                        if bits & (1 << (4 - x)) != 0 {
                            for dy in 0..2 {
                                for dx in 0..2 {
                                    image.put_pixel(
                                        (24 + character * 16 + x * 2 + dx) as u32,
                                        (38 + row * 37 + y * 2 + dy) as u32,
                                        Luma([25]),
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        image
    }
    #[test]
    fn estimates_small_positive_and_negative_text_angles() {
        for angle in [-4.5, -3., -1., -0.5, 0.5, 1., 3., 4.5] {
            let image = rotate_expanded(&text(), angle);
            let estimated = estimate(&image).unwrap_or_else(|| panic!("abstained at {angle}"));
            assert!((estimated.correction_degrees + angle).abs() <= 0.5, "{angle}: {estimated:?}");
            let (corrected, _) = prepare(&image).unwrap();
            assert!(
                estimate(&corrected).is_none(),
                "residual at {angle}: {:?}",
                estimate(&corrected)
            );
        }
    }
    #[test]
    fn blank_platen_single_rule_and_upright_text_abstain() {
        for grey in [0, 40, 180, 255] {
            assert!(estimate(&GrayImage::from_pixel(760, 180, Luma([grey]))).is_none());
        }
        let mut border = GrayImage::from_pixel(760, 180, Luma([235]));
        for x in 0..760 {
            border.put_pixel(x, 80, Luma([20]));
        }
        assert!(estimate(&rotate_expanded(&border, 3.)).is_none());
        assert!(estimate(&text()).is_none());
    }
    #[test]
    fn near_upright_noise_and_platen_frame_abstain() {
        for angle in [-0.2, 0., 0.2] {
            assert!(estimate(&rotate_expanded(&text(), angle)).is_none(), "near zero {angle}");
        }
        let mut random = 0x12345678u32;
        let noise = GrayImage::from_fn(760, 180, |_, _| {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            Luma([(random & 255) as u8])
        });
        assert!(estimate(&noise).is_none());
        let border = GrayImage::from_fn(760, 180, |x, y| {
            Luma([if !(12..748).contains(&x) || !(12..168).contains(&y) { 20 } else { 235 }])
        });
        for angle in [-3., -0.5, 0.5, 3.] {
            assert!(estimate(&rotate_expanded(&border, angle)).is_none(), "border {angle}");
        }
    }
    #[test]
    fn expanded_rotation_retains_all_corners_and_uses_bright_padding() {
        let mut image = GrayImage::from_pixel(120, 60, Luma([235]));
        for &(x, y) in &[(0, 0), (116, 0), (0, 56), (116, 56)] {
            for dy in 0..4 {
                for dx in 0..4 {
                    image.put_pixel(x + dx, y + dy, Luma([20]));
                }
            }
        }
        for angle in [-5., 5.] {
            let rotated = rotate_expanded(&image, angle);
            assert!(rotated.width() > image.width() && rotated.height() > image.height());
            for (xmin, xmax, ymin, ymax) in [
                (0, rotated.width() / 2, 0, rotated.height() / 2),
                (rotated.width() / 2, rotated.width(), 0, rotated.height() / 2),
                (0, rotated.width() / 2, rotated.height() / 2, rotated.height()),
                (rotated.width() / 2, rotated.width(), rotated.height() / 2, rotated.height()),
            ] {
                let dark = (ymin..ymax)
                    .flat_map(|y| (xmin..xmax).map(move |x| (x, y)))
                    .filter(|&(x, y)| rotated.get_pixel(x, y).0[0] < 120)
                    .count();
                assert!(dark >= 12, "lost corner at {angle}: {dark}");
            }
            assert_eq!(rotated.get_pixel(0, 0).0[0], 235);
        }
    }
}
