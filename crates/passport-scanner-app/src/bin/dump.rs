//! Print what the unit's EEPROM actually contains, as this code parses it.
//!
//! ```text
//! cargo run -p passport-scanner-app --bin passport-scanner-dump
//! ```

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(not(windows))]
    {
        eprintln!("This build has no transport for the current platform.");
        Ok(())
    }

    #[cfg(windows)]
    {
        use passport_scanner_capture::CameraSource;
        use passport_scanner_device::{Scanner, parse_tables, protocol::VENDOR_ID};
        use passport_scanner_transport_ks::KsTransport;

        let (transport, info) = KsTransport::open_first(VENDOR_ID)?;
        println!("device {:04X}:{:04X}\n", info.vendor, info.product);

        // The camera is not needed for this, but Scanner wants one.
        let camera = CameraSource::open_named("scanner", 640, 480)
            .or_else(|_| CameraSource::open(0, 640, 480))?;
        let mut scanner = Scanner::new(transport, camera);

        let block = scanner.read_eeprom(0x4000, 0x260)?;
        println!("EEPROM 0x4000 header: {:02X?}", &block[..16]);

        let tables = parse_tables(&block[0x0D..])?;
        println!("\n{} tables parsed from 0x400D:", tables.len());
        for (i, t) in tables.iter().enumerate() {
            println!(
                "  table {i}: {:>3} entries, terminator {}",
                t.writes.len(),
                t.terminator.map_or("end".to_owned(), |m| format!("{m:#04X}"))
            );
            let tail: Vec<String> = t
                .writes
                .iter()
                .rev()
                .take(3)
                .rev()
                .map(|w| format!("{:04X}={:04X}", w.register, w.value))
                .collect();
            println!("           last: {}", tail.join(" "));
        }

        scanner.initialise()?;
        println!(
            "
common table (first) - AEC/AGC and mode registers:"
        );
        for w in &scanner.calibration().common {
            if matches!(w.register, 0x3500..=0x350C | 0x3406 | 0x5001 | 0x5003 | 0x3A00..=0x3A1F) {
                println!("  {:04X} = {:04X}", w.register, w.value);
            }
        }
        println!(
            "
full common table ({} entries):",
            scanner.calibration().common.len()
        );
        let all: Vec<String> = scanner
            .calibration()
            .common
            .iter()
            .map(|w| format!("{:04X}={:02X}", w.register, w.value))
            .collect();
        for chunk in all.chunks(8) {
            println!("  {}", chunk.join("  "));
        }
        let c = scanner.calibration();
        println!("\nchosen tables: white {} entries, infrared {}", c.white.len(), c.infrared.len());
        for (name, table) in [("white", &c.white), ("infrared", &c.infrared)] {
            let key: Vec<String> = table
                .iter()
                .filter(|w| matches!(w.register, 0x3402 | 0x3501 | 0x350B))
                .map(|w| format!("{:04X}={:04X}", w.register, w.value))
                .collect();
            println!("  {name:9} exposure/gain: {}", key.join(" "));
        }

        Ok(())
    }
}
