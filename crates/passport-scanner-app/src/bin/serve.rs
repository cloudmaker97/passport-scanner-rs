//! Standalone browser UI using the same engine as the Passport Scanner desktop app.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut requested = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--profile" => {
                requested = Some(
                    args.next().ok_or("--profile requires general, identity or identity-bright")?,
                )
            }
            "--help" | "-h" => {
                println!(
                    "passport-scanner-serve [--profile general|identity|identity-bright]\nProfile precedence: CLI, PASSPORT_SCANNER_PROFILE, saved preference, General."
                );
                return Ok(());
            }
            _ => return Err(format!("Unknown argument: {arg}").into()),
        }
    }
    let profile = passport_scanner_app::preferences::startup_profile(
        requested.as_deref(),
        std::env::var("PASSPORT_SCANNER_PROFILE").ok().as_deref(),
        &passport_scanner_app::preferences::path()?,
    )?;
    let host = passport_scanner_app::desktop_host::Host::with_profile(profile)?;
    let _web = host.standalone_web()?;
    println!("Passport Scanner web interface: http://127.0.0.1:8733");
    loop {
        std::thread::park();
    }
}
