//! Native session and ownership of optional local services.
use crate::{
    local_http::{self, Endpoint, Server},
    preferences::CaptureSelection,
    service::Engine,
};
use passport_scanner_core::{Error, Result};
use passport_scanner_protocol::{Command, Event};
use std::{
    net::TcpListener,
    path::Path,
    process::{Child, Command as Process},
    sync::{Arc, Mutex, mpsc::Receiver},
};

/// One scanner session; web and MCP start disabled.
#[derive(Debug)]
pub struct Host {
    engine: Arc<Engine>,
    api: Option<Server>,
    web: Mutex<Option<Child>>,
    mcp: Mutex<Option<Server>>,
}
impl Host {
    /// The application always captures with the factory General profile; the
    /// others remain for the diagnostic tools that take `--profile`.
    pub fn start() -> Result<Self> {
        Self::with_profile(CaptureSelection::General)
    }
    pub fn with_profile(profile: CaptureSelection) -> Result<Self> {
        let listener = TcpListener::bind(local_http::API_ADDRESS).map_err(|e| {
            Error::Rejected(format!(
                "Cannot start scanner session on {}. Close the other Passport Scanner session first: {e}",
                local_http::API_ADDRESS
            ))
        })?;
        let engine = Arc::new(Engine::start(profile)?);
        let api = Server::start(listener, engine.clone(), Endpoint::Api)?;
        Ok(Self { engine, api: Some(api), web: Mutex::new(None), mcp: Mutex::new(None) })
    }
    pub fn send(&self, command: Command) -> Result<()> {
        self.engine.send(command)
    }
    pub fn subscribe(&self) -> Receiver<String> {
        self.engine.subscribe()
    }
    pub fn state(&self) -> Event {
        self.engine.state()
    }
    pub fn export_pdf(&self, document_id: &str, path: &Path) -> Result<()> {
        crate::export::write_document_pdf(&self.engine.scans(), document_id, path)
    }
    pub fn web_running(&self) -> bool {
        let mut web = self.web.lock().expect("web child");
        let running = web.as_mut().is_some_and(|child| matches!(child.try_wait(), Ok(None)));
        if !running {
            *web = None;
        }
        running
    }
    pub fn set_web_enabled(&self, enabled: bool) -> Result<()> {
        let mut web = self.web.lock().expect("web child");
        if let Some(child) = web.as_mut() {
            if child.try_wait()?.is_some() {
                *web = None;
            }
        }
        if enabled && web.is_none() {
            drop(TcpListener::bind(local_http::WEB_ADDRESS)?);
            let exe = crate::sibling_tool(if cfg!(windows) {
                "passport-scanner-web.exe"
            } else {
                "passport-scanner-web"
            })?;
            let mut process = Process::new(&exe);
            process
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                process.creation_flags(0x0800_0000);
            }
            let mut child = process.spawn().map_err(|e| {
                Error::Rejected(format!(
                    "Cannot start {}. Build or install passport-scanner-web alongside the desktop: {e}",
                    exe.display()
                ))
            })?;
            for _ in 0..20 {
                if child.try_wait()?.is_some() {
                    return Err(Error::Rejected(
                        "Web interface exited during startup (port may be busy).".into(),
                    ));
                }
                if std::net::TcpStream::connect(local_http::WEB_ADDRESS).is_ok() {
                    *web = Some(child);
                    return Ok(());
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Rejected("Web interface did not start listening.".into()));
        }
        if !enabled && let Some(mut child) = web.take() {
            if child.try_wait()?.is_none() {
                child.kill()?;
            }
            child.wait()?;
        }
        Ok(())
    }
    pub fn mcp_running(&self) -> bool {
        self.mcp.lock().expect("MCP server").is_some()
    }
    pub fn set_mcp_enabled(&self, enabled: bool) -> Result<()> {
        let mut server = self.mcp.lock().expect("MCP server");
        if enabled && server.is_none() {
            *server = Some(Server::start(
                TcpListener::bind(local_http::MCP_ADDRESS)?,
                self.engine.clone(),
                Endpoint::Mcp,
            )?);
        } else if !enabled {
            *server = None;
        }
        Ok(())
    }
    pub fn standalone_web(&self) -> Result<Server> {
        Server::start(
            TcpListener::bind(local_http::WEB_ADDRESS)?,
            self.engine.clone(),
            Endpoint::Web,
        )
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.set_web_enabled(false);
        let _ = self.set_mcp_enabled(false);
        self.api.take();
        self.engine.stop();
    }
}
