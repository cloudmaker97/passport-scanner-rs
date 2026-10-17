//! Talk to an attached scanner and print what it says.
//!
//! This is the end-to-end check that the transport, the request layout and the
//! command encoding all agree with the hardware. It only reads.
//!
//! ```text
//! cargo run -p passport-scanner-transport-ks --example probe
//! ```

use passport_scanner_core::VendorTransport;
use passport_scanner_device::protocol::{
    SEL_CONTROL, SEL_EEPROM, VENDOR_ID, eeprom, eeprom_payload, eeprom_word, key,
};
use passport_scanner_transport_ks::{KsTransport, enumerate, ksp::XU_DATA_LEN};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let found = enumerate(VENDOR_ID)?;
    if found.is_empty() {
        println!("No scanner with vendor {VENDOR_ID:#06x} is attached.");
        return Ok(());
    }

    for f in &found {
        println!(
            "{:04X}:{:04X}  interface {:?}  {}",
            f.info.vendor,
            f.info.product,
            f.info.interface.unwrap_or(0),
            f.info.instance.as_deref().unwrap_or("-")
        );
    }

    let mut t = KsTransport::open(&found[0].device_path)?;
    println!("\nopened {}\n", found[0].device_path);

    // Document sensors. Bit 0 of the value byte is set when covered.
    for (name, k) in [("sensor A", key::SENSOR_A), ("sensor B", key::SENSOR_B)] {
        let mut buf = vec![0u8; XU_DATA_LEN];
        buf[0] = k;
        match t.get(SEL_CONTROL, &mut buf) {
            Ok(()) => println!(
                "{name:9} key {k:#04X} -> {:02X} {:02X}   {}",
                buf[0],
                buf[1],
                if buf[1] & 1 == 1 { "covered" } else { "clear" }
            ),
            Err(e) => println!("{name:9} key {k:#04X} -> {e}"),
        }
    }

    // Calibration stamp. The shipped software caches these two words beside its
    // calibration files to tell whether they still belong to this unit.
    println!();
    let mut words = Vec::new();
    for (name, addr) in [("stamp high", eeprom::STAMP_HI), ("stamp low", eeprom::STAMP_LO)] {
        let mut buf = eeprom_payload(addr, XU_DATA_LEN);
        t.get(SEL_EEPROM, &mut buf)?;
        let word = eeprom_word(&buf).expect("reply is long enough");
        println!("{name:11} {addr:#06X} -> {word:08X}");
        words.push(word);
    }
    println!(
        "\ncalibration stamp {:016X}",
        passport_scanner_device::calibration_stamp(words[0], words[1])
    );

    // One exchange returns a run, not a byte, so the whole calibration block
    // comes back at once.
    let mut buf = eeprom_payload(0x3FB8, XU_DATA_LEN);
    t.get(SEL_EEPROM, &mut buf)?;
    println!("\nEEPROM 0x3FB8 onward:");
    for (i, chunk) in buf[1..33].chunks(4).enumerate() {
        println!(
            "  {:#06X}  {}",
            0x3FB8 + i * 4,
            chunk.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
        );
    }

    Ok(())
}
