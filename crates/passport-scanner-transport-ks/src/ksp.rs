//! The request structure sent with `IOCTL_KS_PROPERTY`.
//!
//! This module is pure byte layout and has no platform dependency, so the
//! encoding — the part that is easy to get silently wrong — is testable
//! anywhere, including against bytes captured from the shipped driver.

/// `IOCTL_KS_PROPERTY`, observed carrying every vendor command of the original
/// driver.
pub const IOCTL_KS_PROPERTY: u32 = 0x002F_0003;

/// The extension unit's property set, read off the wire from the shipped
/// driver: `{AAB8F252-8ED1-4972-8CED-96B17F04408B}`.
pub const XU_PROPERTY_SET: Guid = Guid {
    data1: 0xAAB8_F252,
    data2: 0x8ED1,
    data3: 0x4972,
    data4: [0x8C, 0xED, 0x96, 0xB1, 0x7F, 0x04, 0x40, 0x8B],
};

/// The property within that set.
pub const XU_PROPERTY_ID: u32 = 2;

/// The extension unit's node in the filter's topology.
pub const XU_NODE_ID: u32 = 3;

/// Size of the data buffer the driver exchanges with the unit.
///
/// Thirty-five bytes, as observed; the command layer above only ever fills the
/// first few, and the rest stays zero.
pub const XU_DATA_LEN: usize = 35;

/// Translate the sensor selector's big-endian register/value pair to the
/// bridge's two-stage I2C write. etLib2!FUN_180010400 first selects the
/// register with opcode 0, then writes its value with opcode 2.
/// The high-level command payload is NOT itself a USB extension-unit packet.
#[must_use]
pub fn sensor_write_packets(payload: &[u8]) -> Option<[[u8; XU_DATA_LEN]; 2]> {
    register_write_packets(0, payload)
}

/// Sensor selector 0 uses opcodes 0/2; bridge selector 1 uses opcodes 1/3.
/// Both accept a big-endian address/value pair and emit little-endian words.
#[must_use]
pub fn register_write_packets(selector: u8, payload: &[u8]) -> Option<[[u8; XU_DATA_LEN]; 2]> {
    if payload.len() < 4 || selector > 1 {
        return None;
    }
    let mut packets = [[0u8; XU_DATA_LEN]; 2];
    packets[0][0] = selector;
    packets[0][1] = payload[1];
    packets[0][2] = payload[0];
    packets[1][0] = selector + 2;
    packets[1][1] = payload[3];
    packets[1][2] = payload[2];
    Some(packets)
}

/// `KSPROPERTY_TYPE_GET`.
pub const TYPE_GET: u32 = 0x0000_0001;
/// `KSPROPERTY_TYPE_SET`.
pub const TYPE_SET: u32 = 0x0000_0002;
/// `KSPROPERTY_TYPE_TOPOLOGY` — addresses a node rather than the filter.
pub const TYPE_TOPOLOGY: u32 = 0x1000_0000;

/// A Windows GUID in its native field layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Guid {
    /// First four bytes, little-endian on the wire.
    pub data1: u32,
    /// Next two bytes, little-endian on the wire.
    pub data2: u16,
    /// Next two bytes, little-endian on the wire.
    pub data3: u16,
    /// Final eight bytes, in order.
    pub data4: [u8; 8],
}

impl Guid {
    /// The sixteen bytes as they appear in a request.
    #[must_use]
    pub fn to_bytes(self) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[0..4].copy_from_slice(&self.data1.to_le_bytes());
        out[4..6].copy_from_slice(&self.data2.to_le_bytes());
        out[6..8].copy_from_slice(&self.data3.to_le_bytes());
        out[8..16].copy_from_slice(&self.data4);
        out
    }
}

/// Which way a property request goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Read the property.
    Get,
    /// Write the property.
    Set,
}

impl Direction {
    /// The flags word for this direction, addressing a topology node.
    #[must_use]
    pub const fn flags(self) -> u32 {
        TYPE_TOPOLOGY
            | match self {
                Self::Get => TYPE_GET,
                Self::Set => TYPE_SET,
            }
    }
}

/// Build the 32-byte `KSP_NODE` request for the extension unit.
///
/// Layout is `KSIDENTIFIER { GUID set; u32 id; u32 flags }` followed by the node
/// id and a reserved word.
#[must_use]
pub fn node_request(direction: Direction) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf[0..16].copy_from_slice(&XU_PROPERTY_SET.to_bytes());
    buf[16..20].copy_from_slice(&XU_PROPERTY_ID.to_le_bytes());
    buf[20..24].copy_from_slice(&direction.flags().to_le_bytes());
    buf[24..28].copy_from_slice(&XU_NODE_ID.to_le_bytes());
    // bytes 28..32 stay zero: Reserved
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensor_write_uses_address_then_value_in_bridge_byte_order() {
        let packets = sensor_write_packets(&[0x35, 0x01, 0x00, 0x2b]).unwrap();
        assert_eq!(&packets[0][..3], &[0x00, 0x01, 0x35]);
        assert_eq!(&packets[1][..3], &[0x02, 0x2b, 0x00]);
        assert!(packets.iter().all(|p| p[3..].iter().all(|b| *b == 0)));
        assert!(sensor_write_packets(&[0x35, 1, 0]).is_none());
    }

    #[test]
    fn bridge_write_selects_register_separately_from_sensor_address_space() {
        let packets = register_write_packets(1, &[0, 7, 0x12, 0x60]).unwrap();
        assert_eq!(&packets[0][..3], &[1, 7, 0]);
        assert_eq!(&packets[1][..3], &[3, 0x60, 0x12]);
        assert!(packets.iter().all(|p| p[3..].iter().all(|b| *b == 0)));
        assert!(register_write_packets(2, &[0; 4]).is_none());
    }

    /// Captured from the shipped driver during a real scan, for a read.
    #[rustfmt::skip]
    const OBSERVED_GET: [u8; 32] = [
        0x52, 0xf2, 0xb8, 0xaa, 0xd1, 0x8e, 0x72, 0x49, 0x8c, 0xed, 0x96, 0xb1, 0x7f, 0x04, 0x40, 0x8b,
        0x02, 0x00, 0x00, 0x00,
        0x01, 0x00, 0x00, 0x10,
        0x03, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
    ];

    /// The same request, for a write.
    #[rustfmt::skip]
    const OBSERVED_SET: [u8; 32] = [
        0x52, 0xf2, 0xb8, 0xaa, 0xd1, 0x8e, 0x72, 0x49, 0x8c, 0xed, 0x96, 0xb1, 0x7f, 0x04, 0x40, 0x8b,
        0x02, 0x00, 0x00, 0x00,
        0x02, 0x00, 0x00, 0x10,
        0x03, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn get_request_matches_the_bytes_the_driver_sends() {
        assert_eq!(node_request(Direction::Get), OBSERVED_GET);
    }

    #[test]
    fn set_request_matches_the_bytes_the_driver_sends() {
        assert_eq!(node_request(Direction::Set), OBSERVED_SET);
    }

    #[test]
    fn guid_serialises_with_the_first_three_fields_byte_swapped() {
        // The classic trap: the first three fields are little-endian, the last
        // eight bytes are not.
        assert_eq!(&XU_PROPERTY_SET.to_bytes()[..4], &[0x52, 0xf2, 0xb8, 0xaa]);
        assert_eq!(
            &XU_PROPERTY_SET.to_bytes()[8..],
            &[0x8c, 0xed, 0x96, 0xb1, 0x7f, 0x04, 0x40, 0x8b]
        );
    }

    #[test]
    fn flags_carry_the_topology_bit() {
        assert_eq!(Direction::Get.flags(), 0x1000_0001);
        assert_eq!(Direction::Set.flags(), 0x1000_0002);
    }
}
