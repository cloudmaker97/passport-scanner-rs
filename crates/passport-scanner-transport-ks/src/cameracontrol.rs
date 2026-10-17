//! The standard camera-control properties, as distinct from the vendor unit.
//!
//! A UVC device carries two different sets of knobs, and they are reached
//! through different property sets. The brightness, contrast and gamma a
//! capture stack lists are *video processing* properties; exposure and iris
//! are *camera control* ones, and a stack that does not probe the second set
//! reports no exposure at all — which is how a device that plainly has one can
//! look as though it has not.
//!
//! The shipped configuration names `IRISValue` per illumination, so the
//! original software sets exactly these.

use crate::ksp::Guid;

/// `PROPSETID_VIDCAP_CAMERACONTROL`.
pub const CAMERA_CONTROL_SET: Guid = Guid {
    data1: 0xC6E1_3370,
    data2: 0x30AC,
    data3: 0x11D0,
    data4: [0xA1, 0x8C, 0x00, 0xA0, 0xC9, 0x11, 0x89, 0x56],
};

/// Which camera-control property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CameraProperty {
    /// Pan.
    Pan,
    /// Tilt.
    Tilt,
    /// Roll.
    Roll,
    /// Zoom.
    Zoom,
    /// Exposure time.
    Exposure,
    /// Iris, the aperture.
    Iris,
    /// Focus.
    Focus,
}

impl CameraProperty {
    /// The property's id within the set.
    #[must_use]
    pub const fn id(self) -> u32 {
        match self {
            Self::Pan => 0,
            Self::Tilt => 1,
            Self::Roll => 2,
            Self::Zoom => 3,
            Self::Exposure => 4,
            Self::Iris => 5,
            Self::Focus => 6,
        }
    }
}

/// Let the device choose the value.
pub const FLAG_AUTO: u32 = 0x0001;
/// Use the value given.
pub const FLAG_MANUAL: u32 = 0x0002;

/// `KSPROPERTY_TYPE_GET`.
const TYPE_GET: u32 = 0x0000_0001;
/// `KSPROPERTY_TYPE_SET`.
const TYPE_SET: u32 = 0x0000_0002;
/// `KSPROPERTY_TYPE_BASICSUPPORT` — asks for the range rather than the value.
pub const TYPE_BASICSUPPORT: u32 = 0x0000_0200;

/// Size of a `KSPROPERTY_CAMERACONTROL_S`: the identifier, then value, flags
/// and capabilities.
pub const STRUCT_LEN: usize = 24 + 12;

/// Build the request for one camera-control property.
///
/// Unlike the vendor unit, this addresses the filter itself rather than a node,
/// so there is no topology bit and no node id.
#[must_use]
pub fn request(property: CameraProperty, kind: Kind) -> [u8; 24] {
    let mut buf = [0u8; 24];
    buf[0..16].copy_from_slice(&CAMERA_CONTROL_SET.to_bytes());
    buf[16..20].copy_from_slice(&property.id().to_le_bytes());
    buf[20..24].copy_from_slice(&kind.flags().to_le_bytes());
    buf
}

/// What a request is asking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Read the current value.
    Get,
    /// Write a value.
    Set,
    /// Read the supported range.
    Range,
}

impl Kind {
    const fn flags(self) -> u32 {
        match self {
            Self::Get => TYPE_GET,
            Self::Set => TYPE_SET,
            Self::Range => TYPE_BASICSUPPORT | TYPE_GET,
        }
    }
}

/// Build the data buffer that carries a value and its flags.
#[must_use]
pub fn payload(value: i32, flags: u32) -> [u8; STRUCT_LEN] {
    let mut buf = [0u8; STRUCT_LEN];
    // The data buffer repeats the identifier, then carries the value.
    buf[0..16].copy_from_slice(&CAMERA_CONTROL_SET.to_bytes());
    buf[24..28].copy_from_slice(&value.to_le_bytes());
    buf[28..32].copy_from_slice(&flags.to_le_bytes());
    buf
}

/// Read a value and its flags back out of a reply.
#[must_use]
pub fn parse(reply: &[u8]) -> Option<(i32, u32)> {
    let value = i32::from_le_bytes(reply.get(24..28)?.try_into().ok()?);
    let flags = u32::from_le_bytes(reply.get(28..32)?.try_into().ok()?);
    Some((value, flags))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposure_sits_where_the_standard_puts_it() {
        assert_eq!(CameraProperty::Exposure.id(), 4);
        assert_eq!(CameraProperty::Iris.id(), 5);
    }

    #[test]
    fn a_request_names_the_camera_control_set() {
        let r = request(CameraProperty::Exposure, Kind::Set);
        assert_eq!(&r[..4], &[0x70, 0x33, 0xE1, 0xC6], "GUID is byte-swapped in its first field");
        assert_eq!(u32::from_le_bytes(r[16..20].try_into().unwrap()), 4);
        assert_eq!(u32::from_le_bytes(r[20..24].try_into().unwrap()), TYPE_SET);
    }

    #[test]
    fn a_request_carries_no_topology_bit() {
        // The vendor unit addresses a node; this addresses the filter, and
        // setting the topology bit here makes the driver look for a node that
        // is not there.
        for kind in [Kind::Get, Kind::Set, Kind::Range] {
            let flags = u32::from_le_bytes(
                request(CameraProperty::Exposure, kind)[20..24].try_into().unwrap(),
            );
            assert_eq!(flags & 0x1000_0000, 0, "{kind:?} must not be a topology request");
        }
    }

    #[test]
    fn a_value_round_trips_through_the_payload() {
        let p = payload(-7, FLAG_MANUAL);
        assert_eq!(parse(&p), Some((-7, FLAG_MANUAL)));
    }

    #[test]
    fn a_short_reply_is_refused_rather_than_read_past() {
        assert_eq!(parse(&[0u8; 20]), None);
    }
}
