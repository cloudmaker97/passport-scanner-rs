//! Newline-delimited stdio MCP bridge to the enabled desktop MCP endpoint.
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

const MAX_MESSAGE: u64 = 1_048_576;

#[derive(Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    New,
    AwaitingInitialized,
    Ready,
}

fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

fn dispatch(
    request: Value,
    phase: &mut Phase,
    mut forward: impl FnMut(&Value) -> Result<Option<Value>, ()>,
) -> Option<Value> {
    // Reuse the handler's envelope/parameter validation before lifecycle checks.
    // It operates on an empty snapshot here, so it cannot expose any scan data.
    if let Some(reply) = passport_scanner_app::mcp::handle(request.clone(), &[])
        && reply.get("error").is_some()
    {
        return Some(reply);
    }
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str);
    let initialized_notification = id.is_none() && method == Some("notifications/initialized");
    if initialized_notification && *phase != Phase::AwaitingInitialized {
        return None;
    }
    if method == Some("initialize") && *phase != Phase::New {
        return id.map(|id| rpc_error(id, -32600, "This connection is already initialized."));
    }
    if *phase != Phase::Ready
        && !matches!(method, Some("initialize" | "notifications/initialized" | "ping"))
    {
        return id.map(|id| rpc_error(id, -32002, "Initialize the MCP connection first."));
    }
    match forward(&request) {
        Ok(reply) => {
            if method == Some("initialize")
                && reply.as_ref().and_then(|r| r["result"]["protocolVersion"].as_str())
                    == Some(passport_scanner_app::mcp::PROTOCOL_VERSION)
            {
                *phase = Phase::AwaitingInitialized;
            }
            if initialized_notification {
                *phase = Phase::Ready;
            }
            reply
        }
        Err(()) => id.map(|id| {
            rpc_error(
                id,
                -32000,
                "Open Passport Scanner and enable its MCP server (127.0.0.1:8735).",
            )
        }),
    }
}

fn run(
    mut input: impl BufRead,
    mut output: impl Write,
    mut forward: impl FnMut(&Value) -> Result<Option<Value>, ()>,
) -> io::Result<()> {
    let mut phase = Phase::New;
    loop {
        let mut line = Vec::new();
        let count =
            std::io::Read::take(&mut input, MAX_MESSAGE + 1).read_until(b'\n', &mut line)?;
        if count == 0 {
            return Ok(());
        }
        if count as u64 > MAX_MESSAGE {
            writeln!(output, "{}", rpc_error(Value::Null, -32600, "MCP message exceeds 1 MiB."))?;
            output.flush()?;
            // Close this input rather than allocating an unbounded line or
            // treating its remaining bytes as unrelated messages.
            return Ok(());
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let reply = match serde_json::from_slice(&line) {
            Ok(request) => dispatch(request, &mut phase, &mut forward),
            Err(_) => Some(passport_scanner_app::mcp::parse_error()),
        };
        if let Some(reply) = reply {
            writeln!(output, "{reply}")?;
            output.flush()?;
        }
    }
}

fn main() -> io::Result<()> {
    run(io::stdin().lock(), io::stdout().lock(), |request| {
        let bytes = serde_json::to_vec(request).map_err(|_| ())?;
        passport_scanner_app::local_http::mcp_bridge(&bytes).map_err(|_| ())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn initialize() -> Value {
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}})
    }
    fn call() -> Value {
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})
    }
    fn notification() -> Value {
        json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    }
    fn forward(request: &Value) -> Result<Option<Value>, ()> {
        Ok(passport_scanner_app::mcp::handle(request.clone(), &[]))
    }

    #[test]
    fn readiness_requires_successful_initialize_then_notification() {
        let mut phase = Phase::New;
        assert!(dispatch(notification(), &mut phase, forward).is_none());
        assert_eq!(phase, Phase::New);
        assert_eq!(dispatch(call(), &mut phase, forward).unwrap()["error"]["code"], -32002);
        assert_eq!(
            dispatch(initialize(), &mut phase, |_| Err(())).unwrap()["error"]["code"],
            -32000
        );
        assert_eq!(phase, Phase::New);
        assert!(dispatch(initialize(), &mut phase, forward).unwrap().get("result").is_some());
        assert_eq!(phase, Phase::AwaitingInitialized);
        assert_eq!(dispatch(call(), &mut phase, forward).unwrap()["error"]["code"], -32002);
        assert!(dispatch(notification(), &mut phase, forward).is_none());
        assert_eq!(phase, Phase::Ready);
        assert!(dispatch(call(), &mut phase, forward).unwrap()["result"]["tools"].is_array());
        assert_eq!(dispatch(initialize(), &mut phase, forward).unwrap()["error"]["code"], -32600);
    }

    #[test]
    fn stdin_stdout_roundtrip_is_json_only_and_notifications_are_silent() {
        let input = format!("not-json\n{{}}\n{}\n{}\n{}\n", initialize(), notification(), call());
        let mut output = Vec::new();
        run(io::Cursor::new(input), &mut output, forward).unwrap();
        let messages: Vec<Value> = std::str::from_utf8(&output)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0]["error"]["code"], -32700);
        assert_eq!(messages[1]["error"]["code"], -32600);
        assert_eq!(messages[2]["result"]["protocolVersion"], "2025-11-25");
        assert!(messages[3]["result"]["tools"].is_array());
    }

    #[test]
    fn oversized_input_is_bounded_and_closes_without_forwarding() {
        let input = vec![b'x'; MAX_MESSAGE as usize + 2];
        let mut output = Vec::new();
        run(io::Cursor::new(input), &mut output, |_| panic!("must not forward")).unwrap();
        let reply: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(reply["error"]["code"], -32600);
    }
}
