//! Pixel processing stages.

use crate::{error::Result, frame::Frame, light::LightSource};

/// A per-pixel, per-channel gain map used for flat-field correction.
///
/// Each coefficient is `target / measured_white`, so correction is a multiplication.
/// Coefficients are stored interleaved in blue-green-red order, matching both the
/// on-disk format and [`crate::frame::PixelFormat::Bgr8`].
#[derive(Debug, Clone)]
pub struct ShadingMap {
    /// Map width in pixels.
    pub width: u32,
    /// Map height in pixels.
    pub height: u32,
    /// The light source this map was measured under.
    pub source: LightSource,
    /// `width * height * 3` gain coefficients.
    pub gains: Vec<f32>,
}

impl ShadingMap {
    /// Gain for one pixel and channel, or `None` if out of range.
    #[must_use]
    pub fn gain(&self, x: u32, y: u32, channel: usize) -> Option<f32> {
        if x >= self.width || y >= self.height || channel >= 3 {
            return None;
        }
        let index = ((y as usize * self.width as usize) + x as usize) * 3 + channel;
        self.gains.get(index).copied()
    }

    /// Does the coefficient count match the declared geometry?
    #[must_use]
    pub fn is_consistent(&self) -> bool {
        self.gains.len() == self.width as usize * self.height as usize * 3
    }
}

/// One stage of the image pipeline.
///
/// Stages are composable and order-dependent; an application builds whatever chain it
/// needs. Adding a correction — deskew, crop, denoise, colour management — means adding
/// an implementation here and inserting it into the chain.
pub trait ImageProcessor: Send {
    /// Human-readable name, used in logs and diagnostics.
    fn name(&self) -> &str;

    /// Transform `frame` in place, or return an error if the stage cannot apply.
    fn process(&self, frame: &mut Frame) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> ShadingMap {
        ShadingMap {
            width: 2,
            height: 2,
            source: LightSource::White,
            gains: vec![
                1.0, 1.1, 1.2, // (0,0)
                2.0, 2.1, 2.2, // (1,0)
                3.0, 3.1, 3.2, // (0,1)
                4.0, 4.1, 4.2, // (1,1)
            ],
        }
    }

    #[test]
    fn indexes_row_major_interleaved() {
        let m = map();
        assert_eq!(m.gain(0, 0, 0), Some(1.0));
        assert_eq!(m.gain(1, 0, 2), Some(2.2));
        assert_eq!(m.gain(0, 1, 0), Some(3.0));
        assert_eq!(m.gain(1, 1, 1), Some(4.1));
    }

    #[test]
    fn rejects_out_of_range() {
        let m = map();
        assert_eq!(m.gain(2, 0, 0), None);
        assert_eq!(m.gain(0, 2, 0), None);
        assert_eq!(m.gain(0, 0, 3), None);
    }

    #[test]
    fn consistency_check() {
        assert!(map().is_consistent());
        let mut broken = map();
        broken.gains.pop();
        assert!(!broken.is_consistent());
    }
}
