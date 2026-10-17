//! Error type shared by every layer.

use std::fmt;

/// Convenience alias used throughout the workspace.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// What went wrong.
///
/// Backends wrap their platform-specific failure in [`Error::Backend`] so that callers
/// never have to match on an OS-specific type.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// No device matched the requested selection.
    #[error("no matching device found")]
    DeviceNotFound,

    /// The device is present but refused the operation.
    #[error("device rejected the operation: {0}")]
    Rejected(String),

    /// A vendor command returned a failure status.
    #[error("vendor command {selector:#04x} failed with status {status:#010x}")]
    VendorCommand { selector: u8, status: u32 },

    /// The device did not answer in time.
    #[error("timed out after {0:?}")]
    Timeout(std::time::Duration),

    /// The hardware does not offer what was asked for.
    #[error("unsupported: {0}")]
    Unsupported(String),

    /// Stored calibration does not belong to this unit, or is malformed.
    #[error("calibration invalid: {0}")]
    Calibration(String),

    /// Input data could not be parsed.
    #[error("malformed data: {0}")]
    Malformed(String),

    /// A platform backend failed. The source carries the original error.
    #[error("backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),

    /// Filesystem or stream I/O failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Error {
    /// Wrap a backend-specific error without leaking its type to callers.
    pub fn backend<E>(err: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Backend(Box::new(err))
    }

    /// Build an [`Error::Unsupported`] from anything printable.
    pub fn unsupported(what: impl fmt::Display) -> Self {
        Self::Unsupported(what.to_string())
    }
}
