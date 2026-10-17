//! The known models.
//!
//! Adding hardware support starts here: give the model its product id and the
//! illumination it is fitted with, and the rest of the stack can already enumerate and
//! describe it.

use passport_scanner_core::{DeviceDescriptor, LightSource};

/// A scanner model in the SecureScan family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Model {
    /// X50 — white only.
    X50,
    /// X100 — white and infrared.
    X100,
    /// X150.
    X150,
    /// X-Mini.
    XMini,
    /// X-Cube.
    XCube,
    /// X-Cube 1000.
    XCube1000,
    /// X-Mini 2000 — white and infrared, 5 MP. The reference device for this project.
    XMini2000,
    /// X60.
    X60,
    /// X160.
    X160,
    /// X280.
    X280,
    /// X-Mini NFC — adds contactless chip reading.
    XMiniNfc,
}

impl Model {
    /// Map a USB product id to a model.
    #[must_use]
    pub const fn from_product_id(product: u16) -> Option<Self> {
        Some(match product {
            0x1505 => Self::X50,
            0x1506 => Self::X100,
            0x1501 => Self::X150,
            0x1A00 => Self::XMini,
            0x1A01 => Self::XCube,
            0x1A06 => Self::XCube1000,
            0x1A05 => Self::XMini2000,
            0x150D => Self::X60,
            0x1508 => Self::X160,
            0x1A0B => Self::X280,
            0x1A0D => Self::XMiniNfc,
            _ => return None,
        })
    }

    /// The USB product id this model enumerates with.
    #[must_use]
    pub const fn product_id(self) -> u16 {
        match self {
            Self::X50 => 0x1505,
            Self::X100 => 0x1506,
            Self::X150 => 0x1501,
            Self::XMini => 0x1A00,
            Self::XCube => 0x1A01,
            Self::XCube1000 => 0x1A06,
            Self::XMini2000 => 0x1A05,
            Self::X60 => 0x150D,
            Self::X160 => 0x1508,
            Self::X280 => 0x1A0B,
            Self::XMiniNfc => 0x1A0D,
        }
    }

    /// Marketing name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::X50 => "X50",
            Self::X100 => "X100",
            Self::X150 => "X150",
            Self::XMini => "X-Mini",
            Self::XCube => "X-Cube",
            Self::XCube1000 => "X-Cube 1000",
            Self::XMini2000 => "X-Mini 2000",
            Self::X60 => "X60",
            Self::X160 => "X160",
            Self::X280 => "X280",
            Self::XMiniNfc => "X-Mini NFC",
        }
    }

    /// A descriptor for this model.
    ///
    /// Only the X-Mini 2000 has been verified against real hardware. The others carry
    /// the illumination mask the shipped configuration declares and should be confirmed
    /// before being trusted.
    #[must_use]
    pub fn descriptor(self) -> DeviceDescriptor {
        let sources = match self {
            Self::X50 => vec![LightSource::White],
            _ => vec![LightSource::White, LightSource::Infrared],
        };
        DeviceDescriptor {
            model: self.name(),
            product: self.product_id(),
            sources,
            sensor: (2592, 1944),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_device_round_trips() {
        let m = Model::from_product_id(0x1A05).expect("X-Mini 2000 is known");
        assert_eq!(m, Model::XMini2000);
        assert_eq!(m.product_id(), 0x1A05);
        assert_eq!(m.name(), "X-Mini 2000");
    }

    #[test]
    fn every_model_round_trips_through_its_product_id() {
        for m in [
            Model::X50,
            Model::X100,
            Model::X150,
            Model::XMini,
            Model::XCube,
            Model::XCube1000,
            Model::XMini2000,
            Model::X60,
            Model::X160,
            Model::X280,
            Model::XMiniNfc,
        ] {
            assert_eq!(Model::from_product_id(m.product_id()), Some(m), "{}", m.name());
        }
    }

    #[test]
    fn unknown_product_ids_are_rejected() {
        assert_eq!(Model::from_product_id(0xDEAD), None);
    }

    #[test]
    fn x50_has_no_infrared() {
        assert_eq!(Model::X50.descriptor().sources, vec![LightSource::White]);
    }
}
