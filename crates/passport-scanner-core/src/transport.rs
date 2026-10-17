//! The vendor control channel.
//!
//! Every scanner command in this family travels as a fixed-size payload addressed by a
//! one-byte selector. On Windows that rides on a UVC extension-unit `KSPROPERTY`; on
//! Linux the same unit is reached through V4L2. Implementations of
//! [`VendorTransport`] hide that difference entirely.

use crate::error::Result;

/// A one-byte vendor command selector.
///
/// Named constants live with the concrete device, not here — this crate stays free of
/// model-specific knowledge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Selector(pub u8);

impl Selector {
    /// The raw byte.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl From<u8> for Selector {
    fn from(value: u8) -> Self {
        Self(value)
    }
}

/// A bidirectional vendor command channel.
///
/// Both directions exchange a caller-sized buffer: the device answers in place, so
/// [`VendorTransport::get`] fills the same buffer the request key was written into.
///
/// Implementations must be usable from one thread at a time; wrap in a mutex if shared.
pub trait VendorTransport: Send {
    /// Send `payload` under `selector`.
    fn set(&mut self, selector: Selector, payload: &[u8]) -> Result<()>;

    /// Exchange `buffer` under `selector`, overwriting it with the device's answer.
    fn get(&mut self, selector: Selector, buffer: &mut [u8]) -> Result<()>;

    /// Payload size this transport expects, in bytes.
    ///
    /// The original stack always marshals 32 bytes regardless of how much is meaningful.
    fn payload_len(&self) -> usize {
        32
    }
}

/// Blanket forwarding so a boxed transport is itself a transport.
impl<T: VendorTransport + ?Sized> VendorTransport for Box<T> {
    fn set(&mut self, selector: Selector, payload: &[u8]) -> Result<()> {
        (**self).set(selector, payload)
    }

    fn get(&mut self, selector: Selector, buffer: &mut [u8]) -> Result<()> {
        (**self).get(selector, buffer)
    }

    fn payload_len(&self) -> usize {
        (**self).payload_len()
    }
}
