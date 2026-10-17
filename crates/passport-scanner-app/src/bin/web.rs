//! Separate web UI process. The desktop remains the only scanner owner.
fn main() -> passport_scanner_core::Result<()> {
    passport_scanner_app::local_http::run_web_proxy()
}
