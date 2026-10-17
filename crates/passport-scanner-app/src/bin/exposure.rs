//! Find and sweep the device's real exposure control.
//!
//! A UVC device carries two sets of knobs in two property sets. The brightness
//! and contrast a capture stack lists are video-processing properties; exposure
//! and iris are camera-control ones, and a stack that does not probe the second
//! set reports no exposure at all. This goes at the second set directly.
//!
//! ```text
//! cargo run -p passport-scanner-app --bin passport-scanner-exposure
//! ```

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(not(windows))]
    {
        eprintln!("This build has no transport for the current platform.");
        Ok(())
    }

    #[cfg(windows)]
    {
        use passport_scanner_device::protocol::VENDOR_ID;
        use passport_scanner_transport_ks::{KsTransport, cameracontrol::CameraProperty};

        let (mut transport, info) = KsTransport::open_first(VENDOR_ID)?;
        println!("device {:04X}:{:04X}\n", info.vendor, info.product);

        println!("{:<10} {:>12} {:>22}", "property", "value", "range");
        for property in [
            CameraProperty::Exposure,
            CameraProperty::Iris,
            CameraProperty::Focus,
            CameraProperty::Zoom,
        ] {
            let current = transport.camera_control(property);
            let range = transport.camera_control_range(property);
            match (current, range) {
                (Ok((value, flags)), Ok((min, max, step))) => {
                    let mode = if flags & 1 == 1 { "auto" } else { "manual" };
                    println!(
                        "{:<10} {value:>8} {mode:<6} {min:>8} to {max:<8} step {step}",
                        format!("{property:?}")
                    );
                }
                (Ok((value, flags)), Err(_)) => {
                    println!(
                        "{:<10} {value:>8} flags {flags:#06x}   (no range)",
                        format!("{property:?}")
                    );
                }
                (Err(e), _) => println!("{:<10}   not carried ({e})", format!("{property:?}")),
            }
        }
        Ok(())
    }
}
