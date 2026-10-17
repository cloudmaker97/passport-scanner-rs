//! The scanner itself.

use crate::{error::Result, light::LightSource};
use std::time::Duration;

/// How a device is addressed on the bus.
///
/// The original stack addresses devices by *index* into an enumeration, not by handle,
/// so that is modelled explicitly rather than hidden behind an opaque handle type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceId {
    /// USB vendor id.
    pub vendor: u16,
    /// USB product id.
    pub product: u16,
    /// Position in the enumeration order.
    pub index: u32,
    /// Serial number, when the device reports one.
    pub serial: Option<String>,
}

/// Static facts about a model.
#[derive(Debug, Clone)]
pub struct DeviceDescriptor {
    /// Marketing name, e.g. `"X-Mini 2000"`.
    pub model: &'static str,
    /// USB product id this descriptor applies to.
    pub product: u16,
    /// Illumination the hardware is fitted with.
    pub sources: Vec<LightSource>,
    /// Native sensor resolution in pixels.
    pub sensor: (u32, u32),
}

/// What one illumination source needs in order to produce a usable frame.
#[derive(Debug, Clone)]
pub struct SourceCapabilities {
    /// The source these settings belong to.
    pub source: LightSource,
    /// How long to wait after switching illumination before the image has settled.
    ///
    /// This is a fixed delay, not a measurement: the hardware gives no ready signal, so
    /// the original driver simply waits and then takes the next frame.
    pub settle: Duration,
}

/// A scanner.
///
/// Implementations own a vendor transport and a frame source and turn them into the
/// handful of operations an application actually needs. Supporting a new model means
/// adding an implementation of this trait; nothing above it changes.
pub trait ScannerDevice: Send {
    /// Static facts about this model.
    fn descriptor(&self) -> &DeviceDescriptor;

    /// How this unit is addressed.
    fn id(&self) -> &DeviceId;

    /// Settings for one illumination source, or `None` if it is not fitted.
    fn capabilities(&self, source: LightSource) -> Option<SourceCapabilities>;

    /// Bring the device into a known state. Call once after opening.
    fn initialise(&mut self) -> Result<()>;

    /// Is a document present?
    fn document_present(&mut self) -> Result<DocumentPresence>;

    /// Select illumination and load that source's sensor configuration.
    ///
    /// After this returns the caller must wait [`SourceCapabilities::settle`] before
    /// taking a frame.
    fn select_source(&mut self, source: LightSource) -> Result<()>;

    /// Turn all illumination off.
    fn lights_off(&mut self) -> Result<()>;

    /// Sound the buzzer for `duration`.
    fn buzz(&mut self, duration: Duration) -> Result<()>;
}

/// Which of the document sensors are covered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentPresence {
    /// Nothing on the platen.
    Absent,
    /// Only the first sensor is covered — typically a card, which is too small to reach
    /// the second.
    Partial,
    /// Both sensors are covered — typically a passport page.
    Full,
}

impl DocumentPresence {
    /// Decode the two sensor bits.
    #[must_use]
    pub const fn from_sensors(first: bool, second: bool) -> Self {
        match (first, second) {
            (false, false) => Self::Absent,
            (true, true) => Self::Full,
            _ => Self::Partial,
        }
    }

    /// Is anything on the platen at all?
    #[must_use]
    pub const fn is_present(self) -> bool {
        !matches!(self, Self::Absent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_decoding() {
        assert_eq!(DocumentPresence::from_sensors(false, false), DocumentPresence::Absent);
        assert_eq!(DocumentPresence::from_sensors(true, false), DocumentPresence::Partial);
        assert_eq!(DocumentPresence::from_sensors(false, true), DocumentPresence::Partial);
        assert_eq!(DocumentPresence::from_sensors(true, true), DocumentPresence::Full);
    }

    #[test]
    fn only_absent_counts_as_empty() {
        assert!(!DocumentPresence::Absent.is_present());
        assert!(DocumentPresence::Partial.is_present());
        assert!(DocumentPresence::Full.is_present());
    }
}
