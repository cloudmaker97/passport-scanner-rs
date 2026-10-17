//! Reading the per-unit calibration the scanner carries in EEPROM.

use crate::protocol::eeprom as map;
use passport_scanner_core::{Error, Result};

/// One `register = value` assignment destined for the image sensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisterWrite {
    /// Sensor register address.
    pub register: u16,
    /// Value to write.
    pub value: u16,
}

impl RegisterWrite {
    /// Decode the four-byte big-endian `RRRRVVVV` form used in EEPROM and on the wire.
    #[must_use]
    pub const fn from_bytes(b: [u8; 4]) -> Self {
        Self { register: u16::from_be_bytes([b[0], b[1]]), value: u16::from_be_bytes([b[2], b[3]]) }
    }
}

/// A contiguous run of register writes, as stored in EEPROM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterTable {
    /// Marker byte that terminated this table, or `None` for the final table.
    pub terminator: Option<u8>,
    /// The writes, in the order they must be applied.
    pub writes: Vec<RegisterWrite>,
}

/// Split a raw EEPROM region into its register tables.
///
/// Tables are packed back to back. Each is terminated either by one of
/// [`map::TABLE_MARKERS`] followed by three zero bytes and a [`map::TABLE_LEAD_IN`]
/// byte, or by an all-ones word at the end of the region.
///
/// `bytes` must start at the first register write, not at the region header.
pub fn parse_tables(bytes: &[u8]) -> Result<Vec<RegisterTable>> {
    let mut tables = Vec::new();
    let mut writes = Vec::new();
    let mut i = 0usize;

    while i + 4 <= bytes.len() {
        let word: [u8; 4] = bytes[i..i + 4].try_into().expect("slice is four bytes");

        // End of the whole region.
        if word == [0xFF, 0xFF, 0xFF, 0xFF] {
            if !writes.is_empty() {
                tables
                    .push(RegisterTable { terminator: None, writes: std::mem::take(&mut writes) });
            }
            return Ok(tables);
        }

        // A table boundary: marker, three zero bytes, then the lead-in.
        if map::TABLE_MARKERS.contains(&word[0]) && word[1..] == [0, 0, 0] {
            let lead_in = bytes.get(i + 4).copied();
            if lead_in != Some(map::TABLE_LEAD_IN) {
                return Err(Error::Malformed(format!(
                    "table marker {:#04x} at offset {i} is not followed by the lead-in byte",
                    word[0]
                )));
            }
            tables.push(RegisterTable {
                terminator: Some(word[0]),
                writes: std::mem::take(&mut writes),
            });
            i += 5;
            continue;
        }

        writes.push(RegisterWrite::from_bytes(word));
        i += 4;
    }

    if !writes.is_empty() {
        tables.push(RegisterTable { terminator: None, writes });
    }
    Ok(tables)
}

/// Combine the two stamp words into the 64-bit calibration token.
///
/// The shipped application stores this next to its calibration files so it can tell
/// whether a cached calibration still belongs to the attached unit.
#[must_use]
pub const fn calibration_stamp(high: u32, low: u32) -> u64 {
    ((high as u64) << 32) | low as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The white register table, read verbatim from a real X-Mini 2000 at `0x41D4`.
    #[rustfmt::skip]
    const WHITE_TABLE: &[u8] = &[
        0x56, 0x80, 0x00, 0x03, 0x56, 0x81, 0x00, 0xE8, 0x56, 0x82, 0x00, 0x00,
        0x56, 0x83, 0x00, 0x5A, 0x56, 0x84, 0x00, 0x15, 0x56, 0x85, 0x00, 0x78,
        0x56, 0x86, 0x00, 0x0B, 0x56, 0x87, 0x00, 0x20, 0x58, 0x07, 0x00, 0x30,
        0x58, 0x08, 0x00, 0x30, 0x58, 0x09, 0x00, 0x08, 0x58, 0x0A, 0x00, 0x0B,
        0x58, 0x0D, 0x00, 0x30, 0x58, 0x0E, 0x00, 0x20, 0x58, 0x0F, 0x00, 0x03,
        0x58, 0x10, 0x00, 0x02, 0x58, 0x11, 0x00, 0x20, 0x58, 0x13, 0x00, 0x30,
        0x58, 0x14, 0x00, 0x30, 0x58, 0x15, 0x00, 0x06, 0x58, 0x16, 0x00, 0x06,
        0x58, 0x19, 0x00, 0x30, 0x58, 0x1A, 0x00, 0x30, 0x58, 0x1B, 0x00, 0x30,
        0x58, 0x1C, 0x00, 0x30, 0x58, 0x1D, 0x00, 0x30, 0x34, 0x02, 0x00, 0x04,
        0x35, 0x01, 0x00, 0x30, 0x35, 0x0B, 0x00, 0x15,
        0xFF, 0xFF, 0xFF, 0xFF,
    ];

    #[test]
    fn parses_the_real_white_table() {
        let tables = parse_tables(WHITE_TABLE).expect("table parses");
        assert_eq!(tables.len(), 1);

        let t = &tables[0];
        assert_eq!(t.terminator, None);
        // Fixed shape: 8 metering-window + 18 lens-shading + AWB + exposure + gain.
        assert_eq!(t.writes.len(), 29);

        assert_eq!(t.writes[0], RegisterWrite { register: 0x5680, value: 0x0003 });
        assert_eq!(t.writes[8], RegisterWrite { register: 0x5807, value: 0x0030 });
        assert_eq!(t.writes[28], RegisterWrite { register: 0x350B, value: 0x0015 });
    }

    #[test]
    fn splits_on_a_marker_and_lead_in() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x35, 0x01, 0x00, 0x30]);
        bytes.extend_from_slice(&[0xA2, 0x00, 0x00, 0x00, 0x74]); // marker + lead-in
        bytes.extend_from_slice(&[0x35, 0x0B, 0x00, 0x40]);
        bytes.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);

        let tables = parse_tables(&bytes).expect("tables parse");
        assert_eq!(tables.len(), 2);
        assert_eq!(tables[0].terminator, Some(0xA2));
        assert_eq!(tables[0].writes.len(), 1);
        assert_eq!(tables[1].terminator, None);
        assert_eq!(tables[1].writes[0].value, 0x0040);
    }

    #[test]
    fn rejects_a_marker_without_its_lead_in() {
        let bytes = [0xA1, 0x00, 0x00, 0x00, 0x99, 0xFF, 0xFF, 0xFF, 0xFF];
        assert!(matches!(parse_tables(&bytes), Err(Error::Malformed(_))));
    }

    #[test]
    fn stamp_matches_the_value_the_driver_caches() {
        // Observed in EEPROM at 0x3FF0/0x3FF4; the shipped app writes this into
        // Stamp.ini as "3f3a555a29524d84".
        assert_eq!(calibration_stamp(0x3F3A_555A, 0x2952_4D84), 0x3F3A_555A_2952_4D84);
    }
}
