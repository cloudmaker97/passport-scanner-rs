//! Capture a reproducible camera-control profile through the production scan path.
//! cargo run --release -p passport-scanner-app --example profile -- DIR COUNT Name=value ...
//! Profile=0 selects General; Profile=1 selects Identity with its matching SHD.
#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use passport_scanner_core::LightSource;
    let args: Vec<_> = std::env::args().skip(1).collect();
    let out = std::path::PathBuf::from(args.first().ok_or("output directory required")?);
    let count: usize = args.get(1).ok_or("count required")?.parse()?;
    let controls: Vec<(String, i64)> = args
        .iter()
        .skip(2)
        .map(|arg| {
            let (name, value) = arg.split_once('=').ok_or("expected Name=value")?;
            Ok((name.to_owned(), value.parse()?))
        })
        .collect::<Result<_, Box<dyn std::error::Error>>>()?;
    std::fs::create_dir_all(&out)?;
    let (mut scanner, model) = passport_scanner_app::open()?;
    let profile = match controls.iter().find(|(name, _)| name == "Profile").map(|(_, v)| *v) {
        None | Some(0) => passport_scanner_device::CaptureProfile::General,
        Some(1) => passport_scanner_device::CaptureProfile::Identity,
        Some(_) => return Err("Profile must be 0 (General) or 1 (Identity)".into()),
    };
    scanner.set_capture_profile(profile)?;
    if let Some((_, lamp)) = controls.iter().find(|(name, _)| name == "Lamp") {
        let enabled = match lamp {
            0 => false,
            1 => true,
            _ => return Err("Lamp must be 0 or 1".into()),
        };
        scanner.set_diagnostic_white_lamp(Some(enabled));
    }
    if let Some((_, exposure)) = controls.iter().find(|(name, _)| name == "ExposureRegister") {
        scanner.set_white_exposure(u8::try_from(*exposure)?);
    }
    scanner.set_white_controls(
        controls
            .iter()
            .filter(|(name, _)| !["ExposureRegister", "Profile", "Lamp"].contains(&name.as_str()))
            .cloned()
            .collect(),
    );
    let shading = match profile {
        passport_scanner_device::CaptureProfile::General => {
            passport_scanner_app::calibration::for_documents(0x07b3, model.product_id())?
        }
        passport_scanner_device::CaptureProfile::Identity => {
            let path = passport_scanner_app::calibration::folder(0x07b3, model.product_id())
                .join(profile.shading_file());
            Some(passport_scanner_imaging::Shading::parse(&std::fs::read(path)?)?)
        }
    };
    std::fs::write(out.join("profile.json"), serde_json::to_vec_pretty(&controls)?)?;
    for run in 1..=count {
        let mut frames = scanner.scan(&[LightSource::White, LightSource::Infrared])?;
        for frame in &frames {
            std::fs::write(out.join(format!("{run:03}-{}-raw.jpg", frame.source)), &frame.data)?;
        }
        passport_scanner_app::enhance(&mut frames, shading.as_ref())?;
        for frame in &frames {
            std::fs::write(out.join(format!("{run:03}-{}.jpg", frame.source)), &frame.data)?;
        }
        println!("{run}/{count} captured and decoded");
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows scanner transport required");
}
