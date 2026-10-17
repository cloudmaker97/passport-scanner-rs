//! Scan whatever is on the platen and write the images out.
//!
//! ```text
//! cargo run -p passport-scanner-app --bin passport-scanner-scan -- [output directory] [--profile general|identity|identity-bright]
//! ```

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(not(windows))]
    {
        eprintln!("This build has no transport for the current platform.");
        Ok(())
    }

    #[cfg(windows)]
    {
        use passport_scanner_core::DocumentPresence;
        use std::{path::PathBuf, time::Instant};

        let args: Vec<_> = std::env::args().skip(1).collect();
        let mut out = PathBuf::from("scans");
        let mut profile = "general";
        let mut positional = false;
        let mut arguments = args.iter();
        while let Some(arg) = arguments.next() {
            if arg == "--profile" {
                profile = arguments.next().ok_or("--profile requires a value")?;
            } else if !arg.starts_with('-') && !positional {
                out = arg.into();
                positional = true;
            } else {
                return Err(format!("unexpected argument: {arg}").into());
            }
        }
        let selection = passport_scanner_app::preferences::parse_profile(profile)?;
        std::fs::create_dir_all(&out)?;

        let started = Instant::now();
        let (mut scanner, model, loaded) = passport_scanner_app::open_with_selection(selection)?;
        println!("{} ready in {:?}", model.name(), started.elapsed());
        println!("capture profile: {profile}; calibration: {:?}", loaded.source);
        println!("calibration stamp {:016X}", scanner.calibration().stamp);
        println!(
            "register tables: {} white, {} infrared",
            scanner.calibration().white.len(),
            scanner.calibration().infrared.len()
        );

        match scanner.document_present()? {
            DocumentPresence::Absent => println!("\nNothing on the platen — scanning anyway."),
            p => println!("\nDocument detected ({p:?})."),
        }

        let sources = model.descriptor().sources;
        println!("Scanning {} source(s)…", sources.len());

        let started = Instant::now();
        let mut frames = scanner.scan(&sources)?;
        let captured = started.elapsed();

        // With PASSPORT_SCANNER_KEEP_RAW set, the originals land beside the
        // processed ones: the pair is what says whether the processing is
        // earning its place.
        if std::env::var_os("PASSPORT_SCANNER_KEEP_RAW").is_some() {
            for frame in &frames {
                let name = out.join(format!(
                    "{}-raw.{}",
                    frame.source,
                    passport_scanner_app::extension(frame)
                ));
                std::fs::write(&name, &frame.data)?;
            }
        }

        let started = Instant::now();
        let shading = loaded.shading;
        if shading.is_none() {
            println!("No flat-field calibration on this machine: the platen is not lit evenly,");
            println!("so the result will vary with where the document sits on it.");
        }
        passport_scanner_app::enhance(&mut frames, shading.as_ref())?;
        println!("captured in {captured:?}, processed in {:?}\n", started.elapsed());

        let recognized = passport_scanner_app::read_document_oriented(&frames);
        if let Ok(result) = &recognized {
            if let Some(angle) = result.rotation {
                if let Err(error) =
                    passport_scanner_app::orientation::orient_frames(&mut frames, angle)
                {
                    println!("Could not apply display orientation: {error}");
                }
            }
        }

        for frame in &frames {
            let name =
                out.join(format!("{}.{}", frame.source, passport_scanner_app::extension(frame)));
            std::fs::write(&name, &frame.data)?;
            let (width, height) = passport_scanner_app::orientation::display_dimensions(frame)
                .unwrap_or((frame.width, frame.height));
            println!(
                "{:>10}  {}x{}  {:>8} bytes  {}",
                frame.source.to_string(),
                width,
                height,
                frame.data.len(),
                name.display()
            );
        }

        // Read whatever the images carry.
        println!();
        match recognized.map(|result| result.document) {
            Ok(doc) if doc.fields.is_empty() => {
                println!("No machine-readable zone found on this document.");
            }
            Ok(doc) => {
                println!("Read from the document:");
                for field in doc.fields.values() {
                    let mark = match field.checksum_ok {
                        Some(true) => "check digit ok",
                        Some(false) => "CHECK DIGIT FAILED",
                        None => "",
                    };
                    println!("  {:>16?}  {:<30} {mark}", field.kind, field.value);
                }
                println!(
                    "\ncheck digits: {}",
                    if doc.checksums_valid() { "all match" } else { "one or more failed" }
                );
            }
            Err(e) => println!("Could not read the document: {e}"),
        }

        Ok(())
    }
}
