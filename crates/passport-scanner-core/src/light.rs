//! Illumination sources.

use std::fmt;

/// An illumination source a scanner may offer.
///
/// The discriminants match the `SupportSource` bitmask the original firmware uses, so a
/// capability word maps straight onto this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LightSource {
    /// Visible white light. On every known model this is the resting state.
    White,
    /// Infrared, used to reveal security features and to read IR-absorbing ink.
    Infrared,
    /// Ultraviolet. Not fitted on every model.
    Ultraviolet,
}

impl LightSource {
    /// Bit this source occupies in a `SupportSource` capability mask.
    #[must_use]
    pub const fn bit(self) -> u8 {
        match self {
            Self::White => 0x01,
            Self::Infrared => 0x02,
            Self::Ultraviolet => 0x04,
        }
    }

    /// Every source, in the order a scan sequence normally uses them.
    #[must_use]
    pub const fn all() -> [Self; 3] {
        [Self::White, Self::Infrared, Self::Ultraviolet]
    }

    /// Decode a `SupportSource` mask into the sources it advertises.
    #[must_use]
    pub fn from_mask(mask: u8) -> Vec<Self> {
        Self::all().into_iter().filter(|s| mask & s.bit() != 0).collect()
    }
}

impl fmt::Display for LightSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::White => "white",
            Self::Infrared => "infrared",
            Self::Ultraviolet => "ultraviolet",
        };
        f.write_str(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_three_is_white_and_infrared() {
        // SupportSource=3 is what the X-Mini 2000 reports: white + IR, no UV.
        assert_eq!(LightSource::from_mask(0x03), vec![LightSource::White, LightSource::Infrared]);
    }

    #[test]
    fn mask_one_is_white_only() {
        assert_eq!(LightSource::from_mask(0x01), vec![LightSource::White]);
    }

    #[test]
    fn empty_mask_yields_nothing() {
        assert!(LightSource::from_mask(0).is_empty());
    }
}
