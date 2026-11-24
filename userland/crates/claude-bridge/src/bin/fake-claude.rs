//! A stand-in for the `claude` CLI, for the bridge's tests: nothing real
//! is asked, nothing billed.
//!
//! Run in a session's folder as the bridge runs claude, it checks its
//! command line is the one the bridge means to give, writes what it was
//! given to `fake-claude-args.json` there, reads the message on standard
//! input, and plays `fake-claude.json` from the same folder: a list of
//! steps, each one of
//!
//! - a stream-json message, printed as it is (`$SESSION` becomes the
//!   session's id),
//! - `{"file": "name"}`: every line of a recording in that folder, printed,
//! - `{"ask": {"tool_name", "input", "tool_use_id"}}`: the permission tool
//!   asked, as Claude Code asks it (the MCP server its `--mcp-config` names
//!   started, `initialize`, `tools/call`), and its answer printed as the
//!   tool's result (denied: an error), and kept in `fake-claude-asked.json`,
//! - `{"sleep_ms": n}`.
//!
//! `fake-claude --approver` is the bridge's real permission tool
//! (`catalyst_claude_bridge::approve::main`), so the tests run that too.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--approver") {
        std::process::exit(catalyst_claude_bridge::approve::main());
    }
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    // The command line the bridge gives, or it says what is wrong.
    let fine = args.first().map(String::as_str) == Some("-p")
        && value("--output-format").as_deref() == Some("stream-json")
        && args.iter().any(|a| a == "--verbose")
        && matches!(value("--permission-mode").as_deref(), Some("default" | "acceptEdits"))
        && value("--permission-prompt-tool").is_some()
        && value("--mcp-config").is_some()
        && (value("--resume").is_some() ^ value("--session-id").is_some());
    if !fine {
        eprintln!("fake-claude: not the command line meant: {args:?}");
        std::process::exit(2);
    }
    let session = value("--resume").or(value("--session-id")).unwrap_or_default();
    let mut prompt = String::new();
    std::io::stdin().read_to_string(&mut prompt).ok();
    let here = std::env::current_dir().unwrap_or_default();
    std::fs::write(here.join("fake-claude-args.json"), json!({"args": args, "prompt": prompt}).to_string()).ok();
    let script: Value = serde_json::from_str(&std::fs::read_to_string(here.join("fake-claude.json")).unwrap_or_default()).unwrap_or(json!([]));
    let mcp: Value = serde_json::from_str(&std::fs::read_to_string(value("--mcp-config").unwrap_or_default()).unwrap_or_default()).unwrap_or_default();
    let out = std::io::stdout();
    let say = |line: &str| {
        let mut o = out.lock();
        let _ = writeln!(o, "{}", line.replace("$SESSION", &session));
        let _ = o.flush();
    };
    let mut asked = Vec::new();
    for step in script.as_array().into_iter().flatten() {
        if let Some(ms) = step["sleep_ms"].as_u64() {
            std::thread::sleep(std::time::Duration::from_millis(ms));
        } else if let Some(file) = step["file"].as_str() {
            for line in std::fs::read_to_string(here.join(file)).unwrap_or_default().lines() {
                say(line);
            }
        } else if step.get("ask").is_some() {
            let a = &step["ask"];
            let decided = ask(&mcp, a);
            let allow = decided["behavior"] == "allow";
            asked.push(decided.clone());
            std::fs::write(here.join("fake-claude-asked.json"), Value::from(asked.clone()).to_string()).ok();
            let id = a["tool_use_id"].as_str().unwrap_or("toolu_x");
            let content = if allow { "done".to_owned() } else { decided["message"].as_str().unwrap_or("denied").to_owned() };
            say(&json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": id, "content": content, "is_error": !allow}]}, "parent_tool_use_id": null, "session_id": "$SESSION", "uuid": format!("r-{id}")}).to_string());
        } else {
            say(&step.to_string());
        }
    }
}

/// The permission tool asked as Claude Code asks it: its MCP server from
/// the config started, then initialize and one tools/call.
fn ask(mcp: &Value, ask: &Value) -> Value {
    let server = mcp["mcpServers"].as_object().and_then(|m| m.values().next()).cloned().unwrap_or_default();
    let mut cmd = Command::new(server["command"].as_str().unwrap_or_default());
    cmd.args(server["args"].as_array().into_iter().flatten().filter_map(Value::as_str));
    for (k, v) in server["env"].as_object().into_iter().flatten() {
        cmd.env(k, v.as_str().unwrap_or_default());
    }
    let Ok(mut child) = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else {
        return json!({"behavior": "deny", "message": "the permission tool would not start"});
    };
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut rpc = |msg: Value| -> Option<Value> {
        writeln!(stdin, "{msg}").ok()?;
        stdin.flush().ok()?;
        msg.get("id")?;
        serde_json::from_str(&lines.next()?.ok()?).ok()
    };
    let init = rpc(json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "fake-claude", "version": "0"}}}));
    rpc(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    let listed = rpc(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}));
    let named = listed.as_ref().and_then(|l| l["result"]["tools"][0]["name"].as_str()).unwrap_or_default().to_owned();
    let called = rpc(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": named, "arguments": ask}}));
    drop(rpc);
    drop(stdin);
    let _ = child.wait();
    let text = called.as_ref().and_then(|c| c["result"]["content"][0]["text"].as_str()).unwrap_or("{}");
    let mut decided: Value = serde_json::from_str(text).unwrap_or(json!({"behavior": "deny", "message": "no answer"}));
    decided["initialized"] = Value::from(init.is_some());
    decided["blocks"] = called.as_ref().and_then(|c| c["result"]["content"].as_array()).map_or(Value::Null, |b| Value::from(b.len()));
    decided
}
