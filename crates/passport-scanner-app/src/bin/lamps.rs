//! Find which control key lights which lamp, by trying them and looking.
//!
//! A lit frame compresses far larger than a dark one, so the JPEG's size is a
//! good enough proxy for brightness to sweep with.
//!
//! ```text
//! cargo run -p passport-scanner-app --bin passport-scanner-lamps
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
        use passport_scanner_core::{FrameSource, Selector, VendorTransport};
        use passport_scanner_device::{
            protocol::{SEL_CONTROL, VENDOR_ID, control_payload},
            scanner::FLUSH_FRAMES,
        };
        use passport_scanner_transport_ks::KsTransport;
        use std::{thread::sleep, time::Duration};

        let (mut transport, _) = KsTransport::open_first(VENDOR_ID)?;
        let mut camera = CameraSource::open_named("scanner", 2592, 1944)
            .or_else(|_| CameraSource::open(0, 2592, 1944))?;
        camera.start()?;

        let width = transport.payload_len();
        let set = |t: &mut KsTransport, k: u8, v: u8| -> Result<(), Box<dyn std::error::Error>> {
            t.set(Selector(SEL_CONTROL.get()), &control_payload(k, v, width))?;
            Ok(())
        };

        let measure = |c: &mut CameraSource| -> Result<usize, Box<dyn std::error::Error>> {
            sleep(Duration::from_millis(450));
            for _ in 0..FLUSH_FRAMES {
                c.next_frame(Duration::from_secs(5))?;
            }
            Ok(c.next_frame(Duration::from_secs(5))?.data.len())
        };

        // Everything off, for a baseline.
        for k in [0xF1u8, 0xF2, 0xF4, 0xF5, 0xF6, 0xF7] {
            let _ = set(&mut transport, k, 0);
        }
        let dark = measure(&mut camera)?;
        println!("baseline, nothing set: {dark:>7} bytes\n");

        for key in [0xF1u8, 0xF2, 0xF4, 0xF5, 0xF6, 0xF7] {
            for value in [1u8, 0] {
                set(&mut transport, key, value)?;
                let size = measure(&mut camera)?;
                let verdict = if size > dark * 2 { "  <== LIT" } else { "" };
                println!("key {key:#04X} = {value}  ->  {size:>7} bytes{verdict}");
            }
            set(&mut transport, key, 0)?;
        }

        for k in [0xF1u8, 0xF2, 0xF4, 0xF5, 0xF6, 0xF7] {
            let _ = set(&mut transport, k, 0);
        }
        Ok(())
    }
}
