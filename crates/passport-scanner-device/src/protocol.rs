//! Wire-level constants for the Plustek SecureScan family.
//!
//! Constants are supported by shipped-driver decompilation and recorded scans.
//! Static evidence and live validation are distinguished in the protocol/evidence docs.

use passport_scanner_core::Selector;

/// USB vendor id used by every model in this family.
pub const VENDOR_ID: u16 = 0x07B3;

/// The general control channel.
pub const SEL_CONTROL: Selector = Selector(0xF0);

/// Sensor I2C register write. Payload is `RRRRVVVV`, big-endian.
pub const SEL_SENSOR_REG: Selector = Selector(0x00);
/// Bridge registers, a separate address space from the image sensor.
pub const SEL_BRIDGE_REG: Selector = Selector(0x01);

/// EEPROM read. Payload is a three-byte address-and-length.
pub const SEL_EEPROM: Selector = Selector(0xEE);

/// Keys carried in byte 0 of a [`SEL_CONTROL`] payload.
///
/// The key space is **direction-dependent**: the same number addresses a sensor when
/// read and an actuator when written. Mixing the two up is the single easiest mistake
/// to make against this protocol.
pub mod key {
    /// Read: document sensor A.
    pub const SENSOR_A: u8 = 0xF1;
    /// Write: General-profile white illumination. Verified by sweeping the keys and watching
    /// which one brightens a frame.
    pub const WHITE_LAMP: u8 = 0xF1;

    /// Read: document sensor B.
    pub const SENSOR_B: u8 = 0xF2;
    /// Write: infrared illumination.
    pub const IR_LAMP: u8 = 0xF2;

    /// Write: buzzer. `01` on, `00` off.
    pub const BUZZER: u8 = 0xF3;

    /// Write: frees the imaging path. With this clear the stream delivers
    /// two-byte frames regardless of illumination, so it gates the sensor
    /// rather than a lamp.
    pub const IMAGING_ENABLE: u8 = 0xF6;

    /// Write: initialisation step, written as `F7 01`.
    pub const INIT_STEP: u8 = 0xF7;

    /// Value meaning "on" for the normally-polarised keys.
    pub const ON: u8 = 0x01;
    /// Value meaning "off" for the normally-polarised keys.
    pub const OFF: u8 = 0x00;
}

/// EEPROM addresses that carry per-unit calibration.
///
/// These are factory trim for one specific unit. Reading them is not optional: the
/// values in `Camera.ini` are only defaults, and on a real unit they differ.
pub mod eeprom {
    /// General-profile white exposure, paired with `Calibration_General.shd`.
    pub const WHITE_EXPOSURE: u16 = 0x3FFC;
    /// ID-light exposure, paired with the separate General_ID shading profile.
    pub const ID_EXPOSURE: u16 = 0x3FEC;
    /// Infrared gain, stored as a ready-made `0x350B` register pair.
    pub const IR_GAIN: u16 = 0x3FF8;
    /// General-profile exposure also read during initialisation.
    pub const INIT_EXPOSURE: u16 = 0x3FFC;
    /// General-profile AWB pairs in sensor-register order 3400 through 3405.
    /// The vendor applies these to both white and infrared before the source table.
    pub const GENERAL_AWB: [u16; 6] = [0x3FE4, 0x3FE0, 0x3FDC, 0x3FD8, 0x3FD4, 0x3FD0];
    /// Separate AWB pairs for the ID-light capture profile.
    pub const ID_AWB: [u16; 6] = [0x3FCC, 0x3FC8, 0x3FC4, 0x3FC0, 0x3FBC, 0x3FB8];
    /// High word of the calibration stamp.
    pub const STAMP_HI: u16 = 0x3FF0;
    /// Low word of the calibration stamp.
    pub const STAMP_LO: u16 = 0x3FF4;

    /// First address of the sensor register tables.
    pub const TABLES_BASE: u16 = 0x4000;
    /// Marker byte ending a register table, followed by three zero bytes.
    pub const TABLE_MARKERS: [u8; 3] = [0xA1, 0xA2, 0xA3];
    /// Byte introducing the next table after a marker.
    pub const TABLE_LEAD_IN: u8 = 0x74;
}

/// Sensor registers this driver touches, by function.
///
/// The numbering is the OmniVision map; the imager is a 5 MP part from that family.
pub mod sensor {
    /// Automatic white balance gains, `0x3400`..=`0x3405`.
    pub const AWB_RANGE: std::ops::RangeInclusive<u16> = 0x3400..=0x3405;
    /// Exposure, high byte of the long-exposure triple.
    pub const EXPOSURE: u16 = 0x3501;
    /// Analogue gain.
    pub const GAIN: u16 = 0x350B;
    /// Auto-exposure metering window, `0x5680`..=`0x5687`.
    pub const AE_WINDOW_RANGE: std::ops::RangeInclusive<u16> = 0x5680..=0x5687;
    /// Lens shading correction coefficients, `0x5807`..=`0x581D`.
    ///
    /// These are the hardware half of flat-field correction; the `.shd` float map is a
    /// second, software-side pass layered on top.
    pub const LENC_RANGE: std::ops::RangeInclusive<u16> = 0x5807..=0x581D;
}

/// Build a [`SEL_CONTROL`] payload that writes `value` to `key`.
#[must_use]
pub fn control_payload(key: u8, value: u8, len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    buf[0] = key;
    buf[1] = value;
    buf
}

/// Build a [`SEL_SENSOR_REG`] payload writing `value` to sensor `register`.
#[must_use]
pub fn sensor_payload(register: u16, value: u16, len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    buf[0] = (register >> 8) as u8;
    buf[1] = register as u8;
    buf[2] = (value >> 8) as u8;
    buf[3] = value as u8;
    buf
}

/// Opcode that asks the unit to read EEPROM.
pub const OP_EEPROM_READ: u8 = 0x04;

/// Build a payload reading EEPROM from `address`.
///
/// The address is little-endian in bytes 1 and 2, behind the opcode. The reply
/// comes back in the same buffer with the opcode still in byte 0 and the EEPROM
/// contents from `address` onward in byte 1 and beyond — so one exchange reads
/// a whole run, not a single byte.
///
/// The shipped driver reads byte by byte and keeps only the first, which is why
/// its own trace shows four exchanges for one 32-bit word.
#[must_use]
pub fn eeprom_payload(address: u16, len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    buf[0] = OP_EEPROM_READ;
    buf[1] = address as u8;
    buf[2] = (address >> 8) as u8;
    buf
}

/// Read a big-endian `u32` out of an EEPROM reply.
///
/// The stored words are register/value pairs and stamp halves, both of which
/// the shipped software treats as big-endian.
#[must_use]
pub fn eeprom_word(reply: &[u8]) -> Option<u32> {
    let b = reply.get(1..5)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_payload_matches_observed_bytes() {
        // "f201" — infrared on — as the original driver writes it.
        let p = control_payload(key::IR_LAMP, key::ON, 32);
        assert_eq!(&p[..2], &[0xF2, 0x01]);
        assert_eq!(p.len(), 32);
        assert!(p[2..].iter().all(|&b| b == 0));
    }

    #[test]
    fn sensor_payload_matches_observed_bytes() {
        // "350B0040" — infrared gain — straight from the scan trace.
        let p = sensor_payload(0x350B, 0x0040, 32);
        assert_eq!(&p[..4], &[0x35, 0x0B, 0x00, 0x40]);
    }

    #[test]
    fn eeprom_payload_matches_observed_bytes() {
        // Captured from the shipped driver reading the calibration stamp at
        // 0x3FF0: opcode, then the address little-endian.
        let p = eeprom_payload(eeprom::STAMP_HI, 32);
        assert_eq!(&p[..3], &[0x04, 0xF0, 0x3F]);
    }

    #[test]
    fn eeprom_reply_decodes_to_the_stamp_this_unit_reports() {
        // The reply observed for that request, which is the high half of
        // Stamp.ini's 3f3a555a29524d84.
        let reply = [0x04, 0x3f, 0x3a, 0x55, 0x5a, 0x29, 0x52, 0x4d, 0x84];
        assert_eq!(eeprom_word(&reply), Some(0x3F3A_555A));
    }

    #[test]
    fn a_short_eeprom_reply_is_rejected_rather_than_padded() {
        assert_eq!(eeprom_word(&[0x04, 0x3f]), None);
    }
}
