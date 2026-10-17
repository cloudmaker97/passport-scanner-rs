//! Windows vendor transport over the UVC extension unit.
//!
//! The scanner needs no driver of its own: it enumerates as a standard UVC
//! camera, and every vendor command the original software sends travels as an
//! `IOCTL_KS_PROPERTY` addressed to an extension-unit node on the capture
//! filter. This crate does exactly that and nothing else.
//!
//! The Linux counterpart reaches the same unit through V4L2 with the same GUID
//! and selector, which is why [`passport_scanner_core::VendorTransport`] sits between
//! them: the layer above never learns which operating system it is on.
//!
//! Unsafe code is confined to this crate. [`ksp`] and [`path`] - the byte
//! layout and the path parsing, which are where mistakes hide - are pure and
//! tested on any host.

pub mod cameracontrol;
pub mod ksp;
pub mod path;

#[cfg(windows)]
mod win;

#[cfg(windows)]
pub use win::{KsTransport, enumerate};

#[cfg(not(windows))]
mod stub {
    use passport_scanner_core::{Error, Result};

    /// Enumerate scanners. Always empty off Windows.
    ///
    /// # Errors
    /// Returns [`Error::Unsupported`] on every non-Windows host.
    pub fn enumerate(_vendor: u16) -> Result<Vec<super::path::PathInfo>> {
        Err(Error::unsupported("the kernel streaming transport needs Windows"))
    }
}

#[cfg(not(windows))]
pub use stub::enumerate;
