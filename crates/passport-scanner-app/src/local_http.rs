//! Loopback-only HTTP endpoints and a separate web-interface proxy process.
use crate::service::Engine;
use passport_scanner_core::{Error, Result};
use std::{
    io::{self, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

pub const API_ADDRESS: &str = "127.0.0.1:8734";
pub const WEB_ADDRESS: &str = "127.0.0.1:8733";
pub const MCP_ADDRESS: &str = "127.0.0.1:8735";
pub const MCP_VERSION: &str = "2025-11-25";

#[derive(Debug, Clone, Copy)]
pub enum Endpoint {
    Api,
    Mcp,
    Web,
}

/// A listener owned by the desktop; dropping it stops only this server.
#[derive(Debug)]
pub struct Server {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Server {
    pub fn start(listener: TcpListener, engine: Arc<Engine>, endpoint: Endpoint) -> Result<Self> {
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread =
            std::thread::Builder::new().name("passport-scanner-http".into()).spawn(move || {
                while !stopped.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if accepted(&stream).is_err() {
                                continue;
                            }
                            let engine = engine.clone();
                            let stopped = stopped.clone();
                            std::thread::spawn(move || {
                                let _ = serve(stream, &engine, endpoint, &stopped);
                            });
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(50))
                        }
                        Err(_) => break,
                    }
                }
            })?;
        Ok(Self { stop, thread: Some(thread) })
    }
}
/// Windows hands out accepted sockets in the listener's non-blocking mode. A
/// multi-megabyte event (a scan's images) then fails with WouldBlock as soon
/// as a browser reads more slowly than it is written, ending the stream.
fn accepted(stream: &TcpStream) -> io::Result<()> {
    stream.set_nonblocking(false)
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Debug)]
struct Request {
    method: String,
    path: String,
    headers: std::collections::BTreeMap<String, String>,
    body: Vec<u8>,
}

fn request(stream: &TcpStream) -> io::Result<Request> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        if bytes.len() >= 16_384 {
            return Err(io::Error::other("HTTP headers too large"));
        }
        let mut byte = [0];
        reader.read_exact(&mut byte)?;
        bytes.push(byte[0]);
    }
    let header = std::str::from_utf8(&bytes).map_err(io::Error::other)?;
    let mut lines = header.split("\r\n");
    let parts: Vec<_> = lines.next().unwrap_or_default().split_whitespace().collect();
    if parts.len() != 3 || parts[2] != "HTTP/1.1" {
        return Err(io::Error::other("Invalid HTTP request"));
    }
    let mut headers = std::collections::BTreeMap::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (key, value) =
            line.split_once(':').ok_or_else(|| io::Error::other("Invalid header"))?;
        if headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_owned()).is_some() {
            return Err(io::Error::other("Duplicate header"));
        }
    }
    if headers.contains_key("transfer-encoding") {
        return Err(io::Error::other("Chunked requests are unsupported"));
    }
    let length = headers
        .get("content-length")
        .map(|v| v.parse::<usize>())
        .transpose()
        .map_err(io::Error::other)?
        .unwrap_or(0);
    if length > 1_048_576 {
        return Err(io::Error::other("Request body too large"));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Request { method: parts[0].into(), path: parts[1].into(), headers, body })
}

fn local_authority(value: &str) -> bool {
    let Some((host, port)) = value.rsplit_once(':') else {
        return false;
    };
    matches!(host, "127.0.0.1" | "localhost" | "[::1]") && port.parse::<u16>().is_ok()
}
fn local_request(request: &Request) -> bool {
    request.headers.get("host").is_some_and(|host| local_authority(host))
        && request
            .headers
            .get("origin")
            .is_none_or(|origin| origin.strip_prefix("http://").is_some_and(local_authority))
}

fn response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

fn serve(
    mut stream: TcpStream,
    engine: &Engine,
    endpoint: Endpoint,
    stopped: &AtomicBool,
) -> io::Result<()> {
    let req = match request(&stream) {
        Ok(req) => req,
        Err(_) => {
            return response(
                &mut stream,
                "400 Bad Request",
                "text/plain",
                b"Invalid or oversized request",
            );
        }
    };
    if !local_request(&req) {
        return response(
            &mut stream,
            "403 Forbidden",
            "text/plain",
            b"Only local origins and hosts are accepted",
        );
    }
    if matches!(endpoint, Endpoint::Mcp) {
        if req.path != "/mcp" {
            return response(&mut stream, "404 Not Found", "text/plain", b"Use /mcp");
        }
        if req.method != "POST" {
            return response(
                &mut stream,
                "405 Method Not Allowed",
                "text/plain",
                b"Use POST; no SSE stream is offered",
            );
        }
        let parsed = serde_json::from_slice::<serde_json::Value>(&req.body);
        let initialize = parsed.as_ref().is_ok_and(|v| v["method"] == "initialize");
        if req.headers.get("mcp-protocol-version").map(String::as_str).unwrap_or(if initialize {
            MCP_VERSION
        } else {
            "2025-03-26"
        }) != MCP_VERSION
        {
            return response(
                &mut stream,
                "400 Bad Request",
                "text/plain",
                b"Unsupported MCP-Protocol-Version; use 2025-11-25",
            );
        }
        if !req
            .headers
            .get("content-type")
            .is_some_and(|v| v.split(';').next() == Some("application/json"))
        {
            return response(
                &mut stream,
                "415 Unsupported Media Type",
                "text/plain",
                b"Use application/json",
            );
        }
        let answer = match parsed {
            Ok(value) => crate::mcp::handle(value, &engine.scans_without_pixels()),
            Err(_) => Some(crate::mcp::parse_error()),
        };
        return match answer {
            Some(value) => {
                response(&mut stream, "200 OK", "application/json", &serde_json::to_vec(&value)?)
            }
            None => response(&mut stream, "202 Accepted", "application/json", b""),
        };
    }
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", path) if matches!(endpoint, Endpoint::Web) && is_asset(path) => {
            serve_asset(&mut stream, path)
        }
        ("GET", "/state") => response(
            &mut stream,
            "200 OK",
            "application/json",
            &serde_json::to_vec(&engine.state())?,
        ),
        ("GET", "/events") => {
            // Subscribing already hands this listener the whole picture.
            // Asking for it again would broadcast every image in the session
            // to every other listener as well.
            let rx = engine.subscribe();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: keep-alive\r\n\r\n"
            )?;
            while !stopped.load(Ordering::Acquire) {
                match rx.recv_timeout(Duration::from_secs(1)) {
                    Ok(event) => write!(stream, "data: {event}\n\n")?,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        stream.write_all(b": heartbeat\n\n")?
                    }
                    Err(_) => break,
                }
                stream.flush()?;
            }
            Ok(())
        }
        ("POST", "/command") => match serde_json::from_slice(&req.body) {
            Ok(command) => match engine.send(command) {
                Ok(()) => response(&mut stream, "204 No Content", "application/json", b""),
                Err(_) => response(
                    &mut stream,
                    "503 Service Unavailable",
                    "text/plain",
                    b"Scanner session stopped",
                ),
            },
            Err(_) => response(&mut stream, "400 Bad Request", "text/plain", b"Invalid command"),
        },
        ("GET", path) if path.starts_with("/document/") && path.ends_with("/pdf") => {
            let Some(id) = path
                .strip_prefix("/document/")
                .and_then(|rest| rest.strip_suffix("/pdf"))
                .filter(|id| !id.is_empty())
            else {
                return response(&mut stream, "404 Not Found", "text/plain", b"Not found");
            };
            match crate::export::document_pdf(&engine.scans(), id) {
                Ok(pdf) => response(&mut stream, "200 OK", "application/pdf", &pdf),
                Err(e) => response(
                    &mut stream,
                    "409 Conflict",
                    "text/plain; charset=utf-8",
                    e.to_string().as_bytes(),
                ),
            }
        }
        _ => response(&mut stream, "404 Not Found", "text/plain", b"Not found"),
    }
}

/// The browser client, built by `package.ps1` into `www/` beside the executable.
/// Only these files are reachable; nothing else in that folder is served.
const ASSETS: [(&str, &str, &str); 4] = [
    ("/", "index.html", "text/html; charset=utf-8"),
    ("/passport-scanner-webui.js", "passport-scanner-webui.js", "text/javascript; charset=utf-8"),
    ("/passport-scanner-webui_bg.wasm", "passport-scanner-webui_bg.wasm", "application/wasm"),
    ("/favicon.png", "favicon.png", "image/png"),
];

/// The application icon, so the tab is recognisable even before a build exists.
const FAVICON: &[u8] = include_bytes!("../../../icon.png");

const NOT_BUILT: &str = concat!(
    "<!doctype html><meta charset=utf-8><title>Passport Scanner</title>",
    "<p>Die Weboberfläche ist nicht gebaut: <code>package.ps1</code> ausführen oder ",
    "<code>PASSPORT_SCANNER_WEBUI_DIR</code> setzen.</p>",
    "<p>The web interface is not built: run <code>package.ps1</code> or set ",
    "<code>PASSPORT_SCANNER_WEBUI_DIR</code>.</p>",
);

/// The asset for a request path; a query such as `/?demo` is for the client.
fn find_asset(path: &str) -> Option<&'static (&'static str, &'static str, &'static str)> {
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    ASSETS.iter().find(|(route, ..)| *route == path)
}

fn is_asset(path: &str) -> bool {
    find_asset(path).is_some()
}

/// The browser client: `PASSPORT_SCANNER_WEBUI_DIR`, else `www` beside this
/// program, else `www` one folder up - where it is when this runs from
/// `tools/` in a release.
fn webui_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("PASSPORT_SCANNER_WEBUI_DIR").map(Into::into).or_else(|| {
        let exe = std::env::current_exe().ok()?;
        let here = exe.parent()?.join("www");
        let above = exe.parent()?.parent().map(|dir| dir.join("www"));
        Some(match above {
            Some(above) if !here.exists() && above.exists() => above,
            _ => here,
        })
    })
}

/// Content type and bytes for a route, or `None` if it is not a client file.
fn asset_from(dir: Option<&Path>, path: &str) -> Option<(&'static str, Vec<u8>)> {
    let (_, file, content_type) = find_asset(path)?;
    match dir.and_then(|dir| std::fs::read(dir.join(file)).ok()) {
        Some(bytes) => Some((content_type, bytes)),
        None if *file == "index.html" => Some((content_type, NOT_BUILT.as_bytes().to_vec())),
        None if *file == "favicon.png" => Some((content_type, FAVICON.to_vec())),
        None => None,
    }
}

fn serve_asset(stream: &mut TcpStream, path: &str) -> io::Result<()> {
    match asset_from(webui_dir().as_deref(), path) {
        Some((content_type, body)) => response(stream, "200 OK", content_type, &body),
        None => response(stream, "404 Not Found", "text/plain", b"Web interface not built"),
    }
}

/// Run the separately owned web process. It never opens the scanner.
pub fn run_web_proxy() -> Result<()> {
    let listener = TcpListener::bind(WEB_ADDRESS)?;
    for stream in listener.incoming() {
        let stream = stream?;
        std::thread::spawn(move || {
            let _ = proxy(stream);
        });
    }
    Ok(())
}
fn proxy(mut stream: TcpStream) -> io::Result<()> {
    let req = match request(&stream) {
        Ok(req) => req,
        Err(_) => {
            return response(&mut stream, "400 Bad Request", "text/plain", b"Invalid request");
        }
    };
    if !local_request(&req) {
        return response(
            &mut stream,
            "403 Forbidden",
            "text/plain",
            b"Only local origins and hosts are accepted",
        );
    }
    if req.method == "GET" && is_asset(&req.path) {
        return serve_asset(&mut stream, &req.path);
    }
    if !(matches!(
        (req.method.as_str(), req.path.as_str()),
        ("GET", "/state" | "/events") | ("POST", "/command")
    ) || (req.method == "GET"
        && req.path.starts_with("/document/")
        && req.path.ends_with("/pdf")))
    {
        return response(&mut stream, "404 Not Found", "text/plain", b"Not found");
    }
    let mut upstream = match TcpStream::connect(API_ADDRESS) {
        Ok(stream) => stream,
        Err(_) => {
            return response(
                &mut stream,
                "503 Service Unavailable",
                "text/plain",
                b"Open Passport Scanner first",
            );
        }
    };
    upstream.set_read_timeout(Some(Duration::from_secs(15)))?;
    upstream.set_write_timeout(Some(Duration::from_secs(10)))?;
    write!(
        upstream,
        "{} {} HTTP/1.1\r\nHost: {API_ADDRESS}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        req.method,
        req.path,
        req.body.len()
    )?;
    upstream.write_all(&req.body)?;
    io::copy(&mut upstream, &mut stream)?;
    Ok(())
}

/// Send one JSON-RPC message to the GUI's enabled MCP server.
pub fn mcp_bridge(body: &[u8]) -> Result<Option<serde_json::Value>> {
    let mut stream = TcpStream::connect(MCP_ADDRESS)?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    write!(
        stream,
        "POST /mcp HTTP/1.1\r\nHost: {MCP_ADDRESS}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: {MCP_VERSION}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    let mut reply = Vec::new();
    stream.take(64 * 1024 * 1024).read_to_end(&mut reply)?;
    let split = reply
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| Error::Malformed("Invalid MCP HTTP response".into()))?;
    let status = std::str::from_utf8(&reply[..split]).map_err(Error::backend)?;
    if status.starts_with("HTTP/1.1 202") {
        return Ok(None);
    }
    if !status.starts_with("HTTP/1.1 200") {
        return Err(Error::Rejected("Local MCP server rejected the request".into()));
    }
    Ok(Some(serde_json::from_slice(&reply[split + 4..]).map_err(Error::backend)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_connections_block_instead_of_failing_with_would_block() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(e) => panic!("{e}"),
            }
        };
        accepted(&stream).unwrap();
        // Nothing is sent: a blocking socket waits for the timeout, a
        // non-blocking one fails at once.
        stream.set_read_timeout(Some(Duration::from_millis(150))).unwrap();
        let started = std::time::Instant::now();
        let error = stream.read(&mut [0; 1]).unwrap_err();
        assert!(started.elapsed() >= Duration::from_millis(100), "{error:?} returned immediately");
    }

    #[test]
    fn serves_only_the_three_client_files_and_explains_a_missing_build() {
        let dir = std::env::temp_dir().join(format!("passport-scanner-www-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), "page").unwrap();
        std::fs::write(dir.join("passport-scanner-webui_bg.wasm"), "wasm").unwrap();
        std::fs::write(dir.join("secret.txt"), "no").unwrap();
        assert_eq!(
            asset_from(Some(&dir), "/"),
            Some(("text/html; charset=utf-8", b"page".to_vec()))
        );
        assert_eq!(
            asset_from(Some(&dir), "/passport-scanner-webui_bg.wasm").unwrap().0,
            "application/wasm"
        );
        assert_eq!(asset_from(Some(&dir), "/?demo").unwrap().1, b"page");
        assert_eq!(asset_from(Some(&dir), "/passport-scanner-webui.js"), None);
        for path in ["/secret.txt", "/index.html", "/../secret.txt", "/www/index.html", "/x?/"] {
            assert_eq!(asset_from(Some(&dir), path), None, "{path}");
        }
        let missing = dir.join("missing");
        assert_eq!(asset_from(Some(&missing), "/").unwrap().1, NOT_BUILT.as_bytes());
        assert_eq!(asset_from(None, "/favicon.png"), Some(("image/png", FAVICON.to_vec())));
        assert_eq!(&FAVICON[1..4], b"PNG");
        assert_eq!(asset_from(None, "/").unwrap().1, NOT_BUILT.as_bytes());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rejects_foreign_origins_and_dns_rebinding_hosts() {
        for (host, origin, expected) in [
            ("127.0.0.1:8735", None, true),
            ("localhost:8735", Some("http://localhost:8733"), true),
            ("evil.test:8735", None, false),
            ("127.0.0.1:8735", Some("https://evil.test"), false),
            ("127.0.0.1:8735", Some("http://localhost:8733/evil"), false),
        ] {
            let mut headers = std::collections::BTreeMap::from([("host".into(), host.into())]);
            if let Some(origin) = origin {
                headers.insert("origin".into(), origin.into());
            }
            assert_eq!(
                local_request(&Request {
                    method: "POST".into(),
                    path: "/mcp".into(),
                    headers,
                    body: vec![]
                }),
                expected
            );
        }
    }
}
