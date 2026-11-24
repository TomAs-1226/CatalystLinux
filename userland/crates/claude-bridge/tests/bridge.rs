//! The bridge end to end, against `fake-claude` (recorded stream-json, no
//! real Claude, nothing billed): pairing and refusing the unpaired, the
//! session list, a session carried on and its replies streamed, a new one
//! begun, permission requests allowed, denied and left to time out, a turn
//! stopped, and unpairing.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use catalyst_claude_bridge::{proto, stream, Bridge, Config, Key};
use serde_json::{json, Value};

const SESSION: &str = "3f0c2a9e-1b2c-4d5e-8f90-a1b2c3d4e5f6";
const FAKE: &str = env!("CARGO_BIN_EXE_fake-claude");

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port()
}

/// A computer with one project and one session in it.
struct Rig {
    root: PathBuf,
    work: PathBuf,
    bridge: Bridge,
    port: u16,
    key: Key,
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.bridge.stop();
        std::fs::remove_dir_all(&self.root).ok();
    }
}

fn rig(name: &str, approval_seconds: u64, script: Value) -> Rig {
    let root = std::env::temp_dir().join(format!("catalyst-bridge-test-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();
    let work = root.join("work").join("shell");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("fake-claude.json"), script.to_string()).unwrap();
    std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/turn-edit.jsonl"), work.join("turn-edit.jsonl")).unwrap();
    let projects = root.join("projects");
    let folder = projects.join("C--work-shell");
    std::fs::create_dir_all(&folder).unwrap();
    let cwd = work.to_string_lossy().to_string();
    let lines = [
        json!({"type": "queue-operation", "operation": "enqueue", "sessionId": SESSION}),
        json!({"type": "user", "cwd": cwd, "sessionId": SESSION, "uuid": "u1", "message": {"role": "user", "content": "the camera timer starts from 3, make it 10"}}),
        json!({"type": "assistant", "cwd": cwd, "sessionId": SESSION, "uuid": "u2", "message": {"id": "msg_0", "role": "assistant", "content": [{"type": "text", "text": "Looking at camera.rs."}, {"type": "tool_use", "id": "toolu_0", "name": "Read", "input": {"file_path": format!("{cwd}\\src\\camera.rs")}}]}}),
        json!({"type": "user", "cwd": cwd, "sessionId": SESSION, "uuid": "u3", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_0", "content": "fn timer() {}"}]}}),
        json!({"type": "ai-title", "aiTitle": "Camera timer from 10", "sessionId": SESSION}),
    ];
    let transcript = folder.join(format!("{SESSION}.jsonl"));
    std::fs::write(&transcript, lines.iter().map(|l| l.to_string() + "\n").collect::<String>()).unwrap();
    // Written a while ago: nobody is working in it on the computer.
    std::fs::File::options().write(true).open(&transcript).unwrap().set_modified(SystemTime::now() - Duration::from_secs(600)).unwrap();
    std::fs::write(folder.join("not-a-session.jsonl"), "{}\n").unwrap();
    let port = free_port();
    let config = Config {
        state_dir: root.join("state"),
        projects_dir: projects,
        claude: FAKE.into(),
        approver: (FAKE.into(), vec!["--approver".into()]),
        port,
        approver_port: free_port(),
        computer: "TEST-PC".into(),
        approval_seconds,
        protect: false,
    };
    let bridge = Bridge::start(config).unwrap();
    let (invite, _code) = bridge.begin_pairing("https://test-pc.tail0000.ts.net").unwrap();
    Rig { root, work, bridge, port, key: invite.key }
}

/// A raw POST, its status and body.
fn post(port: u16, path: &str, body: &str) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(s, "POST {path} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    let (head, body) = out.split_once("\r\n\r\n").unwrap();
    (head[9..12].parse().unwrap(), body.to_owned())
}

/// A sealed request, its opened answer.
fn call(rig: &Rig, key: &Key, op: Value) -> Result<Value, (u16, String)> {
    let (rid, body) = key.seal_request(&op, now_ms()).unwrap();
    let (status, text) = post(rig.port, proto::PATH, &body);
    if status != 200 {
        return Err((status, text));
    }
    Ok(key.open_reply(&rid, &text).expect("a reply sealed to its request"))
}

/// The event stream, opened: its events as they come.
fn events(rig: &Rig, after: u64, epoch: &str) -> Receiver<Value> {
    let key = rig.key.clone();
    let (rid, body) = key.seal_request(&json!({"op": "events", "after": after, "epoch": epoch}), now_ms()).unwrap();
    let mut s = TcpStream::connect(("127.0.0.1", rig.port)).unwrap();
    write!(s, "POST {} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", proto::PATH, body.len()).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut index = 0;
        for line in BufReader::new(s).lines() {
            let Ok(line) = line else { break };
            if let Some(data) = line.strip_prefix("data: ") {
                let v = key.open_event(&rid, index, data).expect("each event sealed to its stream and place");
                index += 1;
                if tx.send(v).is_err() {
                    break;
                }
            }
        }
    });
    rx
}

/// Events until one matches, within `secs`; all of them.
fn until(rx: &Receiver<Value>, secs: u64, done: impl Fn(&Value) -> bool) -> Vec<Value> {
    let end = Instant::now() + Duration::from_secs(secs);
    let mut seen = Vec::new();
    loop {
        match rx.recv_timeout(end.saturating_duration_since(Instant::now())) {
            Ok(v) => {
                let stop = done(&v);
                seen.push(v);
                if stop {
                    return seen;
                }
            }
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => panic!("not seen in {secs} s; saw {seen:#?}"),
        }
    }
}

fn hello(rig: &Rig) -> (String, u64) {
    call(rig, &rig.key, json!({"op": "hello", "name": "catalyst"})).unwrap();
    let list = call(rig, &rig.key, json!({"op": "sessions"})).unwrap();
    (list["epoch"].as_str().unwrap().to_owned(), list["seq"].as_u64().unwrap())
}

fn args_given(rig: &Rig) -> Value {
    serde_json::from_str(&std::fs::read_to_string(rig.work.join("fake-claude-args.json")).unwrap()).unwrap()
}

fn turn_done(e: &Value) -> bool {
    e["ev"] == "turn" && e["state"] != "working"
}

#[test]
fn an_unpaired_request_is_refused() {
    let rig = rig("unpaired", 5, json!([]));
    let stranger = Key::from_bytes([1; 32]);
    assert_eq!(call(&rig, &stranger, json!({"op": "sessions"})).unwrap_err(), (401, "not paired".into()));
    assert_eq!(post(rig.port, proto::PATH, "garbage").0, 401);
    assert_eq!(post(rig.port, proto::PATH, &format!("{}.AAAA", rig.key.id_hex())).0, 401, "the right id, nothing sealed");
    assert_eq!(post(rig.port, "/anything", "").0, 404);
    // The paired key: each request once.
    let (_, body) = rig.key.seal_request(&json!({"op": "hello"}), now_ms()).unwrap();
    assert_eq!(post(rig.port, proto::PATH, &body).0, 200);
    assert_eq!(post(rig.port, proto::PATH, &body).0, 409, "a request played again");
    // A clock far off is told the bridge's time, sealed.
    let (rid, body) = rig.key.seal_request(&json!({"op": "sessions"}), now_ms() - 3_600_000).unwrap();
    let (status, text) = post(rig.port, proto::PATH, &body);
    let told = rig.key.open_reply(&rid, &text).unwrap();
    assert_eq!((status, told["error"].as_str()), (200, Some("clock")));
    assert!(told["now"].as_u64().unwrap().abs_diff(now_ms()) < 5_000);
}

#[test]
fn sessions_are_listed_with_their_project_and_activity() {
    let rig = rig("list", 5, json!([]));
    hello(&rig);
    let list = call(&rig, &rig.key, json!({"op": "sessions"})).unwrap();
    let sessions = list["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1, "{list}");
    let s = &sessions[0];
    assert_eq!((s["id"].as_str(), s["title"].as_str(), s["project"].as_str()), (Some(SESSION), Some("Camera timer from 10"), Some("shell")));
    assert!((now_ms() as i64 / 1000 - s["last"].as_i64().unwrap() - 600).abs() < 30, "its last activity");
    assert_eq!(s["running"], false);
    let projects = call(&rig, &rig.key, json!({"op": "projects"})).unwrap();
    assert_eq!(projects["projects"][0]["name"], "shell");
    // Its history: the prompt, the words, the tool as one line.
    let h = call(&rig, &rig.key, json!({"op": "history", "session": SESSION})).unwrap();
    let rows = h["rows"].as_array().unwrap();
    let kinds: Vec<&str> = rows.iter().map(|r| r["k"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["you", "claude", "tool"], "{h}");
    assert_eq!((rows[2]["text"].as_str(), rows[2]["state"].as_str()), (Some("read camera.rs"), Some("ok")));
    assert!(call(&rig, &rig.key, json!({"op": "history", "session": "../../secret"})).unwrap()["error"].is_string());
}

#[test]
fn a_session_carried_on_streams_its_reply() {
    let rig = rig("resume", 5, json!([{"file": "turn-edit.jsonl"}]));
    let (epoch, seq) = hello(&rig);
    let rx = events(&rig, seq, &epoch);
    let sent = call(&rig, &rig.key, json!({"op": "send", "session": SESSION, "text": "make it 10", "mode": "ask"})).unwrap();
    assert_eq!(sent["ok"], true, "{sent}");
    let seen = until(&rx, 20, turn_done);
    assert_eq!(seen.last().unwrap()["state"], "done", "{seen:#?}");
    // Words as they came, then the whole message in the streaming row's place.
    let appended: String = seen.iter().filter(|e| e["ev"] == "append").map(|e| e["text"].as_str().unwrap()).collect();
    assert_eq!(appended, "The timer starts from 3 instead of 10; I'll fix it.Done: it counts down from 10 now.");
    let rows: Vec<&Value> = seen.iter().filter(|e| e["ev"] == "row").map(|e| &e["row"]).collect();
    assert_eq!(rows[0]["k"], "you");
    assert!(rows.iter().any(|r| r["k"] == "claude" && r["replaces"] == "msg_01Fix:s0" && r["text"] == "The timer starts from 3 instead of 10; I'll fix it."));
    let tool: Vec<&&Value> = rows.iter().filter(|r| r["id"] == "toolu_01Edit").collect();
    assert_eq!((tool[0]["text"].as_str(), tool[0]["state"].as_str()), (Some("edited camera.rs +2 −1"), Some("run")));
    assert_eq!(tool.last().unwrap()["state"], "ok");
    // claude was given the session, the mode, and the message on its input.
    let given = args_given(&rig);
    let a: Vec<&str> = given["args"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
    assert!(a.windows(2).any(|w| w == ["--resume", SESSION]) && a.windows(2).any(|w| w == ["--permission-mode", "default"]), "{a:?}");
    assert_eq!(given["prompt"], "make it 10");
    // Its MCP config is gone once the turn is.
    assert_eq!(std::fs::read_dir(rig.root.join("state/run")).unwrap().count(), 0);
}

#[test]
fn a_new_session_begins_in_a_known_folder_only() {
    let rig = rig("new", 5, json!([{"type": "result", "subtype": "success", "is_error": false, "result": "hi", "session_id": "$SESSION"}]));
    let (epoch, seq) = hello(&rig);
    let rx = events(&rig, seq, &epoch);
    let refused = call(&rig, &rig.key, json!({"op": "send", "cwd": "C:\\Windows", "text": "hello"})).unwrap();
    assert!(refused["error"].as_str().unwrap().contains("folder"));
    let sent = call(&rig, &rig.key, json!({"op": "send", "cwd": rig.work.to_string_lossy(), "text": "hello", "mode": "edits"})).unwrap();
    let id = sent["session"].as_str().unwrap().to_owned();
    assert!(id.len() == 36 && id != SESSION);
    until(&rx, 20, turn_done);
    let a: Vec<String> = args_given(&rig)["args"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_owned()).collect();
    assert!(a.windows(2).any(|w| w[0] == "--session-id" && w[1] == id) && a.windows(2).any(|w| w == ["--permission-mode", "acceptEdits"]), "{a:?}");
    // Never bypass, whatever is asked.
    let bypass = call(&rig, &rig.key, json!({"op": "mode", "session": SESSION, "mode": "bypassPermissions"})).unwrap();
    assert!(bypass["error"].is_string());
}

fn ask_script() -> Value {
    json!([
        {"type": "system", "subtype": "init", "session_id": "$SESSION"},
        {"type": "assistant", "uuid": "a1", "session_id": "$SESSION", "parent_tool_use_id": null, "message": {"id": "msg_1", "role": "assistant", "content": [{"type": "tool_use", "id": "toolu_run", "name": "Bash", "input": {"command": "cargo test -p catalyst-shell"}}]}},
        {"ask": {"tool_name": "Bash", "input": {"command": "cargo test -p catalyst-shell"}, "tool_use_id": "toolu_run"}},
        {"type": "result", "subtype": "success", "is_error": false, "result": "ok", "session_id": "$SESSION"}
    ])
}

fn asked(rig: &Rig) -> Value {
    serde_json::from_str::<Value>(&std::fs::read_to_string(rig.work.join("fake-claude-asked.json")).unwrap()).unwrap()[0].clone()
}

#[test]
fn a_permission_request_goes_to_the_handheld_and_is_allowed() {
    let rig = rig("allow", 20, ask_script());
    let (epoch, seq) = hello(&rig);
    let rx = events(&rig, seq, &epoch);
    call(&rig, &rig.key, json!({"op": "send", "session": SESSION, "text": "run the tests"})).unwrap();
    let seen = until(&rx, 20, |e| e["ev"] == "approval");
    let a = &seen.last().unwrap()["approval"];
    assert_eq!((a["what"].as_str(), a["line"].as_str(), a["where"].as_str()), (Some("run a command"), Some("cargo test -p catalyst-shell"), Some("in shell")));
    assert!(a["until"].as_i64().unwrap() > now_ms() as i64 / 1000);
    // It is in the history too, for a handheld that opens the session now.
    let h = call(&rig, &rig.key, json!({"op": "history", "session": SESSION})).unwrap();
    assert_eq!(h["approvals"][0]["id"], a["id"]);
    assert_eq!(h["running"], true);
    let answered = call(&rig, &rig.key, json!({"op": "answer", "approval": a["id"], "allow": true})).unwrap();
    assert_eq!(answered["ok"], true);
    let seen = until(&rx, 20, turn_done);
    assert!(seen.iter().any(|e| e["ev"] == "approval_done" && e["allowed"] == true && e["why"] == "answered"));
    assert!(seen.iter().any(|e| e["ev"] == "row" && e["row"]["id"] == "toolu_run" && e["row"]["state"] == "ok"));
    let decided = asked(&rig);
    assert_eq!((decided["behavior"].as_str(), decided["blocks"].as_u64(), decided["initialized"].as_bool()), (Some("allow"), Some(1), Some(true)), "{decided}");
    assert_eq!(decided["updatedInput"]["command"], "cargo test -p catalyst-shell", "the input as it was");
    assert!(decided.get("updatedPermissions").is_none(), "one call allowed, never a rule");
    // Answered once only.
    assert_eq!(call(&rig, &rig.key, json!({"op": "answer", "approval": a["id"], "allow": false})).unwrap()["ok"], false);
}

#[test]
fn a_denied_request_is_denied() {
    let rig = rig("deny", 20, ask_script());
    let (epoch, seq) = hello(&rig);
    let rx = events(&rig, seq, &epoch);
    call(&rig, &rig.key, json!({"op": "send", "session": SESSION, "text": "run the tests"})).unwrap();
    let a = until(&rx, 20, |e| e["ev"] == "approval").last().unwrap()["approval"].clone();
    call(&rig, &rig.key, json!({"op": "answer", "approval": a["id"], "allow": false})).unwrap();
    let seen = until(&rx, 20, turn_done);
    assert!(seen.iter().any(|e| e["ev"] == "approval_done" && e["allowed"] == false));
    assert!(seen.iter().any(|e| e["ev"] == "row" && e["row"]["id"] == "toolu_run" && e["row"]["state"] == "denied"), "{seen:#?}");
    let decided = asked(&rig);
    assert_eq!(decided["behavior"], "deny");
    assert!(decided["message"].as_str().unwrap().contains("denied"));
}

#[test]
fn an_unanswered_request_is_denied_in_time() {
    let rig = rig("timeout", 2, ask_script());
    let (epoch, seq) = hello(&rig);
    let rx = events(&rig, seq, &epoch);
    call(&rig, &rig.key, json!({"op": "send", "session": SESSION, "text": "run the tests"})).unwrap();
    let began = Instant::now();
    let seen = until(&rx, 20, |e| e["ev"] == "approval_done");
    assert_eq!((seen.last().unwrap()["allowed"].as_bool(), seen.last().unwrap()["why"].as_str()), (Some(false), Some("timeout")));
    assert!(began.elapsed() >= Duration::from_millis(1500) && began.elapsed() < Duration::from_secs(10));
    until(&rx, 20, turn_done);
    let decided = asked(&rig);
    assert_eq!(decided["behavior"], "deny");
    assert!(decided["message"].as_str().unwrap().starts_with("No answer came from the handheld"));
}

#[test]
fn a_turn_can_be_stopped() {
    let rig = rig("stop", 20, json!([{"type": "system", "subtype": "init", "session_id": "$SESSION"}, {"sleep_ms": 30000}, {"type": "result", "subtype": "success", "is_error": false, "result": "late"}]));
    let (epoch, seq) = hello(&rig);
    let rx = events(&rig, seq, &epoch);
    call(&rig, &rig.key, json!({"op": "send", "session": SESSION, "text": "a long job"})).unwrap();
    until(&rx, 10, |e| e["ev"] == "turn" && e["state"] == "working");
    // Busy meanwhile: a second message waits.
    let busy = call(&rig, &rig.key, json!({"op": "send", "session": SESSION, "text": "and another"})).unwrap();
    assert_eq!(busy["error"], "busy");
    let stopped = Instant::now();
    assert_eq!(call(&rig, &rig.key, json!({"op": "interrupt", "session": SESSION})).unwrap()["ok"], true);
    let seen = until(&rx, 10, turn_done);
    assert_eq!(seen.last().unwrap()["state"], "stopped");
    assert!(stopped.elapsed() < Duration::from_secs(5));
}

#[test]
fn unpairing_and_pairing_again_revoke_the_old_key() {
    let rig = rig("revoke", 5, json!([]));
    hello(&rig);
    assert_eq!(rig.bridge.pair_state().paired.as_deref(), Some("catalyst"));
    // Pairing again: the new key's first request revokes the old one.
    let (invite, _) = rig.bridge.begin_pairing("https://test-pc.tail0000.ts.net").unwrap();
    assert!(call(&rig, &rig.key, json!({"op": "hello"})).is_ok(), "the old key works until the new one is used");
    call(&rig, &invite.key, json!({"op": "hello", "name": "second"})).unwrap();
    assert_eq!(call(&rig, &rig.key, json!({"op": "sessions"})).unwrap_err().0, 401, "revoked");
    // The pairing is kept, and read back by a bridge started again.
    let kept = std::fs::read_to_string(rig.root.join("state/pairing.json")).unwrap();
    assert!(kept.contains(&catalyst_companion_proto::hex(invite.key.bytes())), "unprotected in the test only");
    // Unpairing from the handheld.
    call(&rig, &invite.key, json!({"op": "unpair"})).unwrap();
    assert_eq!(call(&rig, &invite.key, json!({"op": "hello"})).unwrap_err().0, 401);
    assert!(!rig.root.join("state/pairing.json").exists());
}

#[test]
fn the_code_pairs_over_https() {
    let mut rig = rig("code", 5, json!([]));
    let (invite, code) = rig.bridge.begin_pairing("https://test-pc.tail0000.ts.net").unwrap();
    let ask = |code: &str| {
        let (asking, msg) = proto::pairing::Asking::new(code);
        let (status, body) = post(rig.port, proto::PAIR_PATH, &json!({"ask": catalyst_companion_proto::hex(&msg)}).to_string());
        assert_eq!(status, 200, "{body}");
        let v: Value = serde_json::from_str(&body).unwrap();
        let unhex = |k: &str| catalyst_companion_proto::unhex(v[k].as_str().unwrap()).unwrap();
        asking.finish(&unhex("answer"), &unhex("sealed"))
    };
    let wrong = if code == "000000" { "111111" } else { "000000" };
    assert!(ask(wrong).is_none(), "a wrong code opens nothing");
    let (key, computer) = ask(&code).unwrap();
    assert_eq!((key.clone(), computer.as_str()), (invite.key, "TEST-PC"));
    rig.key = key;
    hello(&rig);
    assert_eq!(rig.bridge.pair_state().paired.as_deref(), Some("catalyst"));
    // Paired: the code is spent.
    let (_, msg) = proto::pairing::Asking::new(&code);
    assert_eq!(post(rig.port, proto::PAIR_PATH, &json!({"ask": catalyst_companion_proto::hex(&msg)}).to_string()).0, 403);
}

#[test]
fn the_recording_plays_into_rows() {
    let recording = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/turn-edit.jsonl")).unwrap();
    let outs = stream::play("C:\\work\\shell", &recording);
    assert_eq!(outs[0], stream::Out::Session("$SESSION".into()));
    assert!(matches!(outs.last(), Some(stream::Out::Done { ok: true, .. })));
    let texts: Vec<String> = outs.iter().filter_map(|o| if let stream::Out::Row(r) = o { Some(format!("{}:{}", r["k"].as_str().unwrap(), r["text"].as_str().unwrap())) } else { None }).collect();
    assert!(texts.contains(&"tool:edited camera.rs +2 −1".to_owned()), "{texts:?}");
}
