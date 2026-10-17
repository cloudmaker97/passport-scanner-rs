//! Transport-independent, read-only MCP 2025-11-25 request handling.
//!
//! The host owns lifecycle/session enforcement, local access controls and JSON
//! parsing. In particular, HTTP must validate Origin and protocol headers;
//! stdio must reserve stdout for one JSON-RPC message per line.
//! https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle
//! https://modelcontextprotocol.io/specification/2025-11-25/server/tools

use passport_scanner_protocol::{Scan, Side, Source};
use serde_json::{Value, json};

pub const PROTOCOL_VERSION: &str = "2025-11-25";

/// Handle a parsed JSON-RPC message. Valid notifications never get a response.
/// A transport receiving invalid JSON should return [`parse_error`] instead.
/// This handler never scans, changes configuration, or loads external resources.
pub fn handle(request: Value, scans: &[Scan]) -> Option<Value> {
    let Some(object) = request.as_object() else {
        return Some(error(Value::Null, -32600, "Expected a single JSON-RPC request object"));
    };
    let id = object.get("id").cloned();
    let valid_id = id
        .as_ref()
        .is_none_or(|id| id.is_string() || id.as_i64().is_some() || id.as_u64().is_some());
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || object.get("method").and_then(Value::as_str).is_none()
        || !valid_id
        || object.contains_key("result")
        || object.contains_key("error")
    {
        return Some(error(
            if valid_id { id.unwrap_or(Value::Null) } else { Value::Null },
            -32600,
            "Invalid JSON-RPC request",
        ));
    }
    let method = object["method"].as_str().unwrap();
    let id = id?;
    let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
    if !params.is_object() {
        return Some(error(id, -32602, "params must be an object"));
    }
    let result = match method {
        "initialize" => {
            let valid = params["protocolVersion"].is_string()
                && params["capabilities"].is_object()
                && params["clientInfo"]["name"].is_string()
                && params["clientInfo"]["version"].is_string();
            if !valid {
                return Some(error(
                    id,
                    -32602,
                    "initialize requires protocolVersion, capabilities and clientInfo name/version",
                ));
            }
            // Legacy version negotiation responds with a supported version even
            // when the client's preferred version differs; the client may disconnect.
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "passport-scanner-mrz", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "Read existing local document captures. get_mrz prefers the back side and returns recorded fields and check results. This server cannot start a scan."
            })
        }
        "ping" => json!({}),
        "tools/list" => {
            if params.get("cursor").is_some() {
                return Some(error(id, -32602, "This tool list has no pagination cursor"));
            }
            tools()
        }
        "tools/call" => {
            let Some(name) = params["name"].as_str() else {
                return Some(error(id, -32602, "tools/call requires a tool name"));
            };
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            if !arguments.is_object() {
                return Some(error(id, -32602, "Tool arguments must be an object"));
            }
            match name {
                "list_documents" if arguments.as_object().unwrap().is_empty() => {
                    list_documents(scans)
                }
                "list_documents" => {
                    tool_error("invalid_arguments", "list_documents takes no arguments")
                }
                "get_mrz" => get_mrz(&arguments, scans),
                _ => return Some(error(id, -32602, "Unknown tool")),
            }
        }
        _ => return Some(error(id, -32601, "Method not found")),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// JSON-RPC parse error for malformed transport input, without echoing private data.
pub fn parse_error() -> Value {
    error(Value::Null, -32700, "Parse error")
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tools() -> Value {
    let annotations = json!({"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false});
    json!({"tools": [
        {
            "name": "get_mrz",
            "description": "Read the recorded MRZ for a document, preferring its back side. Omit document_id for the most recently captured document. No scanning or inferred fields.",
            "inputSchema": {"type": "object", "properties": {"document_id": {"type": "string", "minLength": 1}}, "additionalProperties": false},
            "annotations": annotations,
        },
        {
            "name": "list_documents",
            "description": "List local captured document IDs, available sides/images and whether recorded MRZ data exists. Does not return images or personal fields.",
            "inputSchema": {"type": "object", "additionalProperties": false},
            "annotations": annotations,
        }
    ]})
}

fn tool_result(data: Value, is_error: bool) -> Value {
    // Both representations are required for compatibility with clients that
    // do not yet read structuredContent. Keep booleans/nulls/numbers typed.
    json!({"content": [{"type": "text", "text": data.to_string()}], "structuredContent": data, "isError": is_error})
}

fn tool_error(code: &str, message: &str) -> Value {
    tool_result(json!({"error": {"code": code, "message": message}}), true)
}

fn selected_mrz<'a>(scans: &'a [Scan], document: &str) -> Option<&'a Scan> {
    // Prefer a verified zone over failed OCR, then prefer the back side.
    // Never replace missing back data with fields from a different document.
    let has_mrz =
        |s: &&Scan| s.document == document && s.mrz.iter().any(|line| !line.trim().is_empty());
    let verified = |s: &&Scan| s.checks_passed();
    scans
        .iter()
        .filter(has_mrz)
        .filter(verified)
        .find(|s| s.side == Side::Back)
        .or_else(|| scans.iter().filter(has_mrz).filter(verified).find(|s| s.side == Side::Front))
        .or_else(|| scans.iter().filter(has_mrz).find(|s| s.side == Side::Back))
        .or_else(|| scans.iter().filter(has_mrz).find(|s| s.side == Side::Front))
}

fn get_mrz(arguments: &Value, scans: &[Scan]) -> Value {
    if arguments.as_object().unwrap().keys().any(|key| key != "document_id") {
        return tool_error("invalid_arguments", "Only document_id is accepted");
    }
    let document = match arguments.get("document_id") {
        Some(Value::String(id)) if !id.trim().is_empty() => id.as_str(),
        Some(_) => return tool_error("invalid_arguments", "document_id must be a nonempty string"),
        None => match scans.first() {
            Some(scan) => &scan.document,
            None => return tool_error("no_documents", "No document has been captured"),
        },
    };
    if !scans.iter().any(|s| s.document == document) {
        return tool_error("document_not_found", "No captured document has this document_id");
    }
    let Some(scan) = selected_mrz(scans, document) else {
        return tool_error(
            "mrz_not_found",
            "This document has no recorded MRZ. Capture its MRZ side in the scanner application first.",
        );
    };
    let (passed, total) = scan.check_tally();
    tool_result(
        json!({
            "documentId": scan.document,
            "scanId": scan.id,
            "side": scan.side,
            "capturedAt": scan.at,
            "format": scan.format,
            "rawLines": scan.mrz,
        "mrzCheck": scan.mrz_check,
            "fields": scan.fields,
        "checks": {"passed": passed, "total": total, "composite": scan.mrz_check,
            "allPassed": scan.mrz_check.map(|_| scan.checks_passed())}
        }),
        false,
    )
}

fn list_documents(scans: &[Scan]) -> Value {
    let mut seen = std::collections::HashSet::new();
    let mut documents = Vec::new();
    for scan in scans.iter() {
        if !seen.insert(&scan.document) {
            continue;
        }
        let sides: Vec<Value> = [Side::Front, Side::Back]
            .into_iter()
            .filter_map(|side| {
                scans.iter().find(|s| s.document == scan.document && s.side == side).map(|s| {
                    json!({
                        "side": side, "scanId": s.id, "capturedAt": s.at,
                        "sources": s.images.iter().map(|i| i.source).collect::<Vec<_>>()
                    })
                })
            })
            .collect();
        let complete = [Side::Front, Side::Back].into_iter().all(|side| {
            scans.iter().find(|s| s.document == scan.document && s.side == side).is_some_and(|s| {
                [Source::White, Source::Infrared]
                    .iter()
                    .all(|source| s.images.iter().any(|image| &image.source == source))
            })
        });
        documents
            .push(json!({"documentId": scan.document, "latestCaptureAt": scan.at, "sides": sides,
            "hasMrz": selected_mrz(scans, &scan.document).is_some(), "completeForPdf": complete}));
    }
    tool_result(json!({"documents": documents}), false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use passport_scanner_protocol::{Field, FieldKind};

    fn scan(side: Side) -> Scan {
        Scan {
            id: format!("synthetic-{side:?}"),
            document: "synthetic".into(),
            side,
            at: "2026-01-01T00:00:00Z".into(),
            format: Some("TD1".into()),
            mrz: vec!["SYNTHETIC_TEST_ONLY".into()],
            mrz_check: Some(false),
            images: vec![],
            fields: vec![Field {
                kind: FieldKind::DocumentNumber,
                value: "TEST00001".into(),
                check: Some(false),
                span: None,
            }],
        }
    }

    fn call(name: &str, arguments: Value, scans: &[Scan]) -> Value {
        handle(json!({"jsonrpc":"2.0", "id": 7, "method":"tools/call", "params":{"name":name,"arguments":arguments}}), scans).unwrap()
    }

    #[test]
    fn initialization_negotiates_supported_version_and_notifications_are_silent() {
        let result = handle(json!({"jsonrpc":"2.0", "id":"init", "method":"initialize", "params":{
            "protocolVersion":"2099-01-01", "capabilities":{}, "clientInfo":{"name":"test", "version":"1"}
        }}), &[]).unwrap();
        assert_eq!(result["id"], "init");
        assert_eq!(result["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(result["result"]["capabilities"]["tools"]["listChanged"], false);
        assert!(
            handle(json!({"jsonrpc":"2.0", "method":"notifications/initialized"}), &[]).is_none()
        );
        assert!(handle(json!({"jsonrpc":"2.0", "method":"notifications/cancelled", "params":{"requestId":1}}), &[]).is_none());
        assert_eq!(
            handle(json!({"jsonrpc":"2.0", "id":0, "method":"ping"}), &[]).unwrap()["result"],
            json!({})
        );
    }

    #[test]
    fn malformed_requests_and_unknown_tools_have_protocol_errors() {
        for request in [
            json!([]),
            json!({}),
            json!({"jsonrpc":"2.0", "id":null, "method":"ping"}),
            json!({"jsonrpc":"2.0", "id":1.5, "method":"ping"}),
        ] {
            assert_eq!(handle(request, &[]).unwrap()["error"]["code"], -32600);
        }
        assert_eq!(call("unknown", json!({}), &[])["error"]["code"], -32602);
        assert_eq!(
            handle(json!({"jsonrpc":"2.0", "id":1, "method":"unknown"}), &[]).unwrap()["error"]["code"],
            -32601
        );
        assert_eq!(parse_error()["error"]["code"], -32700);
    }

    #[test]
    fn back_mrz_wins_and_fields_checks_and_text_are_not_invented_or_retyped() {
        let mut front = scan(Side::Front);
        front.mrz = vec!["FRONT_SYNTHETIC".into()];
        let back = scan(Side::Back);
        // Front appears most recently but back remains preferred.
        let result = call("get_mrz", json!({"document_id":"synthetic"}), &[front, back]);
        let structured = &result["result"]["structuredContent"];
        assert_eq!(structured["side"], "back");
        assert_eq!(structured["rawLines"][0], "SYNTHETIC_TEST_ONLY");
        assert_eq!(structured["fields"][0]["check"], false);
        assert_eq!(structured["fields"][0]["value"], "TEST00001");
        assert_eq!(
            structured["checks"],
            json!({"passed":0,"total":1,"composite":false,"allPassed":false})
        );
        assert_eq!(
            serde_json::from_str::<Value>(result["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap(),
            *structured
        );
    }

    #[test]
    fn missing_mrz_is_explicit_and_zero_checks_do_not_claim_validity() {
        assert_eq!(
            call("get_mrz", json!({}), &[])["result"]["structuredContent"]["error"]["code"],
            "no_documents"
        );
        let mut s = scan(Side::Front);
        s.fields.clear();
        s.mrz_check = None;
        let result = call("get_mrz", json!({}), &[s.clone()]);
        assert!(result["result"]["structuredContent"]["checks"]["allPassed"].is_null());
        s.mrz.clear();
        assert_eq!(
            call("get_mrz", json!({}), &[s.clone()])["result"]["structuredContent"]["error"]["code"],
            "mrz_not_found"
        );
        assert_eq!(
            call("get_mrz", json!({"document_id":"absent"}), &[s])["result"]["structuredContent"]["error"]
                ["code"],
            "document_not_found"
        );
    }

    #[test]
    fn default_document_uses_protocol_newest_first_order() {
        let mut newest = scan(Side::Front);
        newest.document = "newest".into();
        newest.mrz.clear();
        let older = scan(Side::Back);
        let scans = [newest, older];
        // Do not silently return somebody else's older MRZ when the latest
        // document has not had its MRZ side captured yet.
        assert_eq!(
            call("get_mrz", json!({}), &scans)["result"]["structuredContent"]["error"]["code"],
            "mrz_not_found"
        );
        assert_eq!(
            call("get_mrz", json!({"document_id":"synthetic"}), &scans)["result"]["structuredContent"]
                ["documentId"],
            "synthetic"
        );
        assert_eq!(
            call("list_documents", json!({}), &scans)["result"]["structuredContent"]["documents"]
                [0]["documentId"],
            "newest"
        );
    }

    #[test]
    fn failed_composite_never_verifies_and_valid_front_beats_invalid_back() {
        let mut front = scan(Side::Front);
        front.fields[0].check = Some(true);
        front.mrz_check = Some(true);
        let mut back = scan(Side::Back);
        back.fields[0].check = Some(true);
        back.mrz_check = Some(false);
        let result = call("get_mrz", json!({}), &[back.clone(), front]);
        assert_eq!(result["result"]["structuredContent"]["side"], "front");
        assert_eq!(result["result"]["structuredContent"]["checks"]["allPassed"], true);
        let result = call("get_mrz", json!({}), &[back]);
        assert_eq!(result["result"]["structuredContent"]["checks"]["allPassed"], false);
        assert_eq!(result["result"]["structuredContent"]["checks"]["composite"], false);
    }

    #[test]
    fn list_is_deterministic_and_omits_personal_fields_and_image_bytes() {
        let scans = [scan(Side::Front), scan(Side::Back)];
        let result = call("list_documents", json!({}), &scans);
        let docs = result["result"]["structuredContent"]["documents"].as_array().unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0]["sides"].as_array().unwrap().len(), 2);
        assert_eq!(docs[0]["hasMrz"], true);
        assert_eq!(docs[0]["completeForPdf"], false);
        assert!(!result.to_string().contains("TEST00001"));
        assert_eq!(call("get_mrz", json!({"document_id":7}), &scans)["result"]["isError"], true);
        assert_eq!(
            call("list_documents", json!({"extra":true}), &scans)["result"]["isError"],
            true
        );
    }
}
