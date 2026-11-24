//! Permission requests, Claude Code's to the handheld.
//!
//! Each turn's claude is given `--permission-prompt-tool
//! mcp__catalyst-handheld__approve` and an MCP config that runs [`main`]
//! (`catalyst-companion.exe --claude-approver`) as a stdio MCP server.
//! Whenever Claude Code would ask a person (a command, a file outside the
//! project, the web), it calls that tool with `tool_name`, `input` and
//! `tool_use_id`. The tool asks the bridge on its own local port (never
//! published), with the turn's ticket from its environment; the bridge
//! sends the request to the handheld and waits for its answer, or denies
//! it once [`crate::APPROVAL_SECONDS`] pass.
//!
//! What the tool returns is what Claude Code's documentation asks for: one
//! text block holding `{"behavior": "allow"}` (with the input as it was, so
//! older versions that needed `updatedInput` keep it) or
//! `{"behavior": "deny", "message": …}`, and no `structuredContent`. It
//! never returns `updatedPermissions`: an answer allows one call, never a
//! rule.

use std::io::{BufRead, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::{http, summary, Shared};

/// Inputs up to this size are handed back with an allow (larger ones are
/// left out, as Claude Code's documentation says to).
const ECHO_MOST: usize = 32 * 1024;

/// The bridge's side: one request from the tool, answered when the
/// handheld answers.
pub(crate) fn serve(s: &Arc<Shared>, conn: &TcpStream) {
    let Some(req) = http::read(conn) else { return http::text(conn, 400, "no") };
    if req.method != "POST" || req.path != "/approve" {
        return http::text(conn, 404, "not here");
    }
    let v: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
    let Some((turn, session, cwd)) = s.turn_of_ticket(v["ticket"].as_str().unwrap_or_default()) else {
        return http::text(conn, 401, "no such turn");
    };
    let tool = v["tool_name"].as_str().unwrap_or("a tool").to_owned();
    let tool_use_id = v["tool_use_id"].as_str().unwrap_or_default().to_owned();
    let input = v["input"].clone();
    let asked = summary::asked(&tool, &input, &cwd);
    let id = s.new_approval(&turn, &session, &tool, &tool_use_id, asked);
    let (allow, why) = s.wait_answer(&id);
    let answer = if allow {
        let echo = input.is_object() && input.to_string().len() <= ECHO_MOST;
        if echo {
            json!({"behavior": "allow", "updatedInput": input})
        } else {
            json!({"behavior": "allow"})
        }
    } else {
        s.denied(&turn, &tool_use_id);
        let message = match why {
            "timeout" => format!("No answer came from the handheld within {} minutes, so this was not allowed. Do not try it again unasked; say what you wanted to do.", s.config.approval_seconds / 60),
            "stopped" | "gone" => "The turn was stopped from the handheld.".to_owned(),
            _ => "The user denied this from the handheld. Do not try it again; ask what they would like instead.".to_owned(),
        };
        json!({"behavior": "deny", "message": message})
    };
    http::reply(conn, 200, "application/json", answer.to_string().as_bytes());
}

// ------------------------------------------------------------- the tool ---

fn tool_list() -> Value {
    json!({"tools": [{
        "name": "approve",
        "description": "Catalyst's permission prompt: asks the user on their handheld whether a tool call may run. Claude Code calls it itself; never call it.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "tool_name": {"type": "string", "description": "The tool asking"},
                "input": {"type": "object", "description": "Its input"},
                "tool_use_id": {"type": "string", "description": "Its call's id"}
            },
            "required": ["tool_name", "input"]
        }
    }]})
}

/// The answer to one tools/call: the bridge's decision, as one text block.
fn call(port: u16, ticket: &str, args: &Value, wait: Duration) -> Value {
    let ask = json!({"ticket": ticket, "tool_name": args["tool_name"], "input": args["input"], "tool_use_id": args["tool_use_id"]});
    let decision = http::post_local(port, "/approve", &ask, wait).unwrap_or_else(|why| json!({"behavior": "deny", "message": format!("The handheld could not be asked ({why}), so this was not allowed.")}));
    json!({"content": [{"type": "text", "text": decision.to_string()}]})
}

/// One JSON-RPC message from Claude Code, answered (None for a notification).
fn answer(msg: &Value, port: u16, ticket: &str, wait: Duration) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let result = match msg["method"].as_str().unwrap_or_default() {
        "initialize" => {
            let version = msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18");
            json!({"protocolVersion": version, "capabilities": {"tools": {}}, "serverInfo": {"name": crate::runner::SERVER, "version": env!("CARGO_PKG_VERSION")}})
        }
        "tools/list" => tool_list(),
        "tools/call" if msg["params"]["name"] == "approve" => call(port, ticket, &msg["params"]["arguments"], wait),
        "ping" => json!({}),
        _ => return Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "not here"}})),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// The permission tool, run by Claude Code as a stdio MCP server: it reads
/// the bridge's port and the turn's ticket from its environment (set in
/// the turn's MCP config, never on a command line). Each call is answered
/// on a thread of its own, so calls made side by side wait side by side.
pub fn main() -> i32 {
    let port: u16 = std::env::var("CATALYST_APPROVER_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(crate::APPROVER_PORT);
    let ticket = std::env::var("CATALYST_APPROVER_TICKET").unwrap_or_default();
    // Longer than the bridge's own wait, which ends every request.
    let wait = Duration::from_secs(crate::APPROVAL_SECONDS + 60);
    let out = Arc::new(Mutex::new(std::io::stdout()));
    let stdin = std::io::stdin();
    let mut threads = Vec::new();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let (out, ticket) = (out.clone(), ticket.clone());
        let reply = move || {
            if let Some(a) = answer(&msg, port, &ticket, wait) {
                let mut o = out.lock().unwrap_or_else(|e| e.into_inner());
                let _ = writeln!(o, "{a}");
                let _ = o.flush();
            }
        };
        if line.contains("\"tools/call\"") {
            threads.push(std::thread::spawn(reply));
        } else {
            reply();
        }
    }
    for t in threads {
        let _ = t.join();
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_speaks_mcp() {
        let wait = Duration::from_millis(10);
        let init = answer(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}), 1, "t", wait).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert!(init["result"]["capabilities"]["tools"].is_object());
        assert_eq!(answer(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}), 1, "t", wait), None);
        let list = answer(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}), 1, "t", wait).unwrap();
        assert_eq!(list["result"]["tools"][0]["name"], "approve");
        assert!(list["result"]["tools"][0].get("outputSchema").is_none(), "no structured output");
        // No bridge to ask: denied, as one text block.
        let call = answer(&json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "approve", "arguments": {"tool_name": "Bash", "input": {"command": "ls"}}}}), 9, "t", wait).unwrap();
        let blocks = call["result"]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        let decided: Value = serde_json::from_str(blocks[0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(decided["behavior"], "deny");
        assert!(call["result"].get("structuredContent").is_none());
        assert_eq!(answer(&json!({"jsonrpc": "2.0", "id": 4, "method": "resources/list"}), 1, "t", wait).unwrap()["error"]["code"], -32601);
    }
}
