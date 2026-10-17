//! Standalone browser UI using the same engine as the Passport Scanner desktop app.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(arg) = std::env::args().nth(1) {
        if matches!(arg.as_str(), "--help" | "-h") {
            println!(
                "passport-scanner-serve\nServes the browser interface on http://127.0.0.1:8733."
            );
            return Ok(());
        }
        return Err(format!("Unknown argument: {arg}").into());
    }
    let host = passport_scanner_app::desktop_host::Host::start()?;
    let _web = host.standalone_web()?;
    println!("Passport Scanner web interface: http://127.0.0.1:8733");
    loop {
        std::thread::park();
    }
}
