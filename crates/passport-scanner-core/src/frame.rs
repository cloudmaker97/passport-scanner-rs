//! Image frames and where they come from.

use crate::{error::Result, light::LightSource};
use std::time::Duration;

/// Pixel layout of a [`Frame`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PixelFormat {
    /// 8 bits per pixel, single channel.
    Gray8,
    /// 24 bits per pixel, blue-green-red order. This is what the shading maps use.
    Bgr8,
    /// 24 bits per pixel, red-green-blue order.
    Rgb8,
    /// A complete JPEG image, as delivered by an MJPEG stream.
    Jpeg,
}

impl PixelFormat {
    /// Bytes per pixel, or `None` for compressed formats.
    #[must_use]
    pub const fn bytes_per_pixel(self) -> Option<usize> {
        match self {
            Self::Gray8 => Some(1),
            Self::Bgr8 | Self::Rgb8 => Some(3),
            Self::Jpeg => None,
        }
    }
}

/// One image control a capture source carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Control {
    /// Name as the platform reports it.
    pub name: String,
    /// Lowest accepted value.
    pub min: i64,
    /// Highest accepted value.
    pub max: i64,
    /// Value right now.
    pub value: i64,
}

impl Control {
    /// Whether this control would accept `value`.
    #[must_use]
    pub const fn accepts(&self, value: i64) -> bool {
        self.min <= value && value <= self.max
    }
}

/// One captured image, tagged with the illumination it was taken under.
#[derive(Debug, Clone)]
pub struct Frame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// How `data` is laid out.
    pub format: PixelFormat,
    /// Which lamp was on when this was taken.
    pub source: LightSource,
    /// Raw pixel or JPEG bytes.
    pub data: Vec<u8>,
}

impl Frame {
    /// Distance in bytes between the start of consecutive rows.
    ///
    /// Rows are padded to a four-byte boundary, matching the Windows DIB convention the
    /// original pipeline uses. Returns `None` for compressed formats.
    #[must_use]
    pub fn stride(&self) -> Option<usize> {
        let bpp = self.format.bytes_per_pixel()?;
        Some((self.width as usize * bpp + 3) & !3)
    }

    /// Number of pixels in the image.
    #[must_use]
    pub const fn pixel_count(&self) -> usize {
        self.width as usize * self.height as usize
    }
}

/// A source of image frames - a live camera, or a recorded capture for testing.
///
/// The scan sequence needs exactly two things from a capture backend: the ability to
/// start and stop streaming, and the ability to take the next frame after the
/// illumination has settled.
pub trait FrameSource: Send {
    /// Begin streaming. Idempotent.
    fn start(&mut self) -> Result<()>;

    /// Stop streaming. Idempotent.
    fn stop(&mut self) -> Result<()>;

    /// Return the next frame, waiting at most `timeout`.
    ///
    /// The caller is responsible for having waited out the illumination settling delay
    /// first; this returns whatever the stream delivers next.
    fn next_frame(&mut self, timeout: Duration) -> Result<Frame>;

    /// Set one of the backend's own image controls, such as brightness.
    ///
    /// These sit above any sensor registers: a camera stack that runs its own
    /// exposure loop will override a register write it disagrees with, but it
    /// honours these. A source with no controls says so, and a caller that was
    /// only trying to improve the picture carries on without them.
    ///
    /// # Errors
    /// [`crate::Error::Unsupported`] when the source has no such control.
    fn set_control(&mut self, name: &str, _value: i64) -> Result<()> {
        Err(crate::Error::unsupported(format!("no image control named {name}")))
    }

    /// Put the source back into the state it was in when first opened.
    ///
    /// Some capture stacks configure a device on open in a way that does not
    /// survive the first stream they serve, and a source that knows this about
    /// itself says so here. The default does nothing, which is right for a
    /// source with no such problem - a recorded capture, for instance.
    ///
    /// # Errors
    /// Fails if the source cannot be restored.
    fn reopen(&mut self) -> Result<()> {
        Ok(())
    }

    /// The image controls this source carries, with the range each accepts.
    ///
    /// Empty where a source has none, which is also the right answer for one
    /// that cannot be asked. A caller setting a control needs the range as
    /// well as the name: the values that matter here come from the vendor's
    /// configuration, written in its own scale, and one that falls outside a
    /// control's range means something this code has not established.
    fn controls(&self) -> Vec<Control> {
        Vec::new()
    }

    /// Native device defaults, used before sensor programming to undo
    /// persistent control changes left by diagnostic or other applications.
    fn control_defaults(&self) -> Vec<(String, i64)> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stride_is_padded_to_four_bytes() {
        let frame = Frame {
            width: 649, // deliberately not a multiple of four
            height: 1,
            format: PixelFormat::Bgr8,
            source: LightSource::White,
            data: Vec::new(),
        };
        // 649 * 3 = 1947, which rounds up to 1948.
        assert_eq!(frame.stride(), Some(1948));
    }

    #[test]
    fn compressed_frames_have_no_stride() {
        let frame = Frame {
            width: 2592,
            height: 1944,
            format: PixelFormat::Jpeg,
            source: LightSource::Infrared,
            data: Vec::new(),
        };
        assert_eq!(frame.stride(), None);
    }
}
