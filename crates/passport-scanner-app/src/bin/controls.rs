//! Show the camera controls the device exposes, and sweep one of them.
//!
//! ```text
//! cargo run -p passport-scanner-app --bin passport-scanner-controls            # list
//! cargo run -p passport-scanner-app --bin passport-scanner-controls Exposure   # sweep
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
        use passport_scanner_core::{FrameSource, LightSource, Selector, VendorTransport};
        use passport_scanner_device::{
            protocol::{SEL_CONTROL, VENDOR_ID, control_payload, key},
            scanner::FLUSH_FRAMES,
        };
        use passport_scanner_transport_ks::KsTransport;
        use std::time::Duration;

        let (mut transport, _) = KsTransport::open_first(VENDOR_ID)?;
        let mut camera = CameraSource::open_named("scanner", 2592, 1944)
            .or_else(|_| CameraSource::open(0, 2592, 1944))?;

        println!("camera controls:\n");
        println!("{:<22} {:>8} {:>8} {:>8}", "name", "min", "max", "now");
        for c in camera.controls()? {
            println!("{:<22} {:>8} {:>8} {:>8}", c.name, c.min, c.max, c.value);
        }

        let Some(target) = std::env::args().nth(1) else {
            return Ok(());
        };
        let Some(control) =
            camera.controls()?.into_iter().find(|c| c.name.eq_ignore_ascii_case(&target))
        else {
            eprintln!("\nno control named {target}");
            return Ok(());
        };

        // Light the platen the way a white capture does.
        let width = transport.payload_len();
        let mut set = |k: u8, v: u8| -> passport_scanner_core::Result<()> {
            transport.set(Selector(SEL_CONTROL.get()), &control_payload(k, v, width))
        };
        set(key::IMAGING_ENABLE, 1)?;
        set(key::WHITE_LAMP, 1)?;
        set(key::IR_LAMP, 0)?;
        camera.set_source(LightSource::White);
        camera.start()?;

        println!("\nsweeping {} from {} to {}\n", control.name, control.min, control.max);
        std::fs::create_dir_all("scans/controls")?;
        println!("{:>10}   {:>5}   {:>7}", "value", "mean", "bytes");

        let span = control.max - control.min;
        let steps = 8i64;
        for i in 0..=steps {
            let value = control.min + span * i / steps;
            if camera.set_control(&control.name, value).is_err() {
                println!("{value:>10}   (refused)");
                continue;
            }
            std::thread::sleep(Duration::from_millis(500));
            for _ in 0..FLUSH_FRAMES {
                camera.next_frame(Duration::from_secs(5))?;
            }
            let frame = camera.next_frame(Duration::from_secs(5))?;
            let mean = card_mean(&frame.data)?;
            std::fs::write(format!("scans/controls/{}-{value}.jpg", control.name), &frame.data)?;
            println!("{value:>10}   {mean:>5.0}   {:>7}", frame.data.len());
        }

        let _ = camera.stop();
        set(key::WHITE_LAMP, 0)?;
        set(key::IMAGING_ENABLE, 0)?;
        Ok(())
    }
}

/// Mean brightness over the document, ignoring the dark platen around it.
#[cfg(windows)]
fn card_mean(jpeg: &[u8]) -> Result<f32, Box<dyn std::error::Error>> {
    let grey = image::load_from_memory(jpeg)?.to_luma8();
    let (w, h) = (grey.width(), grey.height());
    let px = grey.as_raw();
    let r = passport_scanner_imaging::crop::bounds(px, w, h, 90)
        .filter(|r| r.plausible(w, h))
        .unwrap_or(passport_scanner_core::Rect::whole(w, h));

    let mut total = 0u64;
    let mut count = 0u64;
    for y in (r.y..r.y + r.h).step_by(4) {
        for x in (r.x..r.x + r.w).step_by(4) {
            total += u64::from(px[(y * w + x) as usize]);
            count += 1;
        }
    }
    Ok(total as f32 / count.max(1) as f32)
}
