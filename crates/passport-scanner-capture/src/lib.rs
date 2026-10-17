//! Frame capture over the platform's camera stack.
//!
//! The scanner streams as an ordinary UVC camera, so capture uses the operating
//! system's own camera stack — Media Foundation on Windows, V4L2 on Linux —
//! rather than anything device-specific. Only the vendor control channel needs
//! to know what this hardware is.
//!
//! Frames are handed on as the JPEG the camera already produced. Decoding a
//! 5 MP frame to count its pixels, only to re-encode it for display, would be
//! work done for nobody.

#![forbid(unsafe_code)]

#[cfg(any(windows, target_os = "linux"))]
mod camera;

#[cfg(any(windows, target_os = "linux"))]
pub use camera::{CameraSource, list_cameras};
// Re-exported so a caller of this crate need not also depend on the core one
// just to name what a control is.
pub use passport_scanner_core::Control;

#[cfg(not(any(windows, target_os = "linux")))]
mod unsupported {
    use passport_scanner_core::{Error, Result};

    /// List attached cameras.
    ///
    /// # Errors
    /// Always, on platforms without a capture backend.
    pub fn list_cameras() -> Result<Vec<(u32, String)>> {
        Err(Error::unsupported("no capture backend for this platform"))
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
pub use unsupported::list_cameras;
