//! One short run against the real `claude` CLI, never run by `cargo test`
//! (it is billed): `cargo test -p catalyst-claude-bridge --test smoke --
//! --ignored`, with `CATALYST_SMOKE_APPROVER` naming the permission tool to
//! run (`catalyst-companion.exe`, given `--claude-approver`).
//!
//! In a throwaway folder: a new session asked to write one small file, its
//! permission request answered yes, the reply streamed, then its history
//! read back from the transcript Claude Code wrote, to see the rows' ids
//! are the stream's.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use catalyst_claude_bridge::{proto, sessions, Bridge, Config, Key};
use serde_json::{json, Value};

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

fn call(port: u16, key: &Key, op: Value) -> Value {
    let (rid, body) = key.seal_request(&op, now_ms()).unwrap();
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(s, "POST {} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", proto::PATH, body.len()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    let (_, text) = out.split_once("\r\n\r\n").unwrap();
    key.open_reply(&rid, text).expect("a sealed reply")
}

fn events(port: u16, key: Key, epoch: String, after: u64) -> Receiver<Value> {
    let (rid, body) = key.seal_request(&json!({"op": "events", "after": after, "epoch": epoch}), now_ms()).unwrap();
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(s, "POST {} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", proto::PATH, body.len()).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut index = 0;
        for line in BufReader::new(s).lines().map_while(Result::ok) {
            if let Some(data) = line.strip_prefix("data: ") {
                let v = key.open_event(&rid, index, data).expect("sealed to its stream");
                index += 1;
                if tx.send(v).is_err() {
                    break;
                }
            }
        }
    });
    rx
}

#[test]
#[ignore = "runs the real claude CLI, which is billed"]
fn real_claude_writes_a_file_once_allowed() {
    let approver = std::env::var("CATALYST_SMOKE_APPROVER").expect("CATALYST_SMOKE_APPROVER: catalyst-companion.exe");
    let claude = catalyst_claude_bridge::runner::find_claude().expect("claude installed");
    let root = std::env::temp_dir().join(format!("catalyst-smoke-{}", std::process::id()));
    let work = root.join("work");
    let projects = root.join("projects");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(projects.join("p")).unwrap();
    // A folder the bridge knows, so a new session may begin in it.
    let known = json!({"type": "user", "cwd": work.to_string_lossy(), "message": {"role": "user", "content": "x"}});
    std::fs::write(projects.join("p").join("00000000-0000-4000-8000-000000000000.jsonl"), format!("{known}\n")).unwrap();
    let port = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    let approver_port = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    let config = Config { state_dir: root.join("state"), projects_dir: projects, claude, approver: (approver.into(), vec!["--claude-approver".into()]), port, approver_port, computer: "SMOKE".into(), approval_seconds: 120, protect: false };
    let bridge = Bridge::start(config).unwrap();
    let (invite, _) = bridge.begin_pairing("https://smoke.invalid").unwrap();
    let key = invite.key;
    call(port, &key, json!({"op": "hello", "name": "smoke"}));
    let list = call(port, &key, json!({"op": "sessions"}));
    let rx = events(port, key.clone(), list["epoch"].as_str().unwrap().to_owned(), list["seq"].as_u64().unwrap());
    let sent = call(port, &key, json!({"op": "send", "cwd": work.to_string_lossy(), "text": "Create a file named hello.txt in this folder containing just the word hi. Do nothing else, then reply with one short sentence.", "mode": "ask"}));
    assert_eq!(sent["ok"], true, "{sent}");
    let session = sent["session"].as_str().unwrap().to_owned();
    let began = Instant::now();
    let mut seen: Vec<Value> = Vec::new();
    loop {
        let e = rx.recv_timeout(Duration::from_secs(240)).expect("events within four minutes");
        println!("event: {}", serde_json::to_string(&e).unwrap().chars().take(300).collect::<String>());
        if e["ev"] == "approval" {
            let a = &e["approval"];
            println!("asked: {} · {} · {}", a["what"], a["line"], a["where"]);
            assert_eq!(call(port, &key, json!({"op": "answer", "approval": a["id"], "allow": true}))["ok"], true);
        }
        let done = e["ev"] == "turn" && e["state"] != "working";
        seen.push(e);
        if done {
            break;
        }
    }
    println!("turn took {:.1} s", began.elapsed().as_secs_f32());
    assert_eq!(seen.last().unwrap()["state"], "done", "{seen:#?}");
    assert!(seen.iter().any(|e| e["ev"] == "approval"), "a permission request came");
    let written = std::fs::read_to_string(work.join("hello.txt")).unwrap_or_default();
    assert_eq!(written.trim(), "hi");
    // The transcript Claude Code wrote, read back as the handheld would.
    let found = sessions::list(&sessions::projects_dir(), &mut sessions::Cache::default()).into_iter().find(|s| s.id == session).expect("its transcript");
    let history = sessions::history(&found, 50);
    let streamed: Vec<String> = seen.iter().filter(|e| e["ev"] == "row" && e["row"]["k"] == "claude" && e["row"]["replaces"].is_string()).map(|e| e["row"]["id"].as_str().unwrap().to_owned()).collect();
    let kept: Vec<String> = history.iter().filter(|r| r["k"] == "claude").map(|r| r["id"].as_str().unwrap().to_owned()).collect();
    println!("streamed ids {streamed:?}\nhistory ids {kept:?}\ntitle {:?}", found.title);
    assert!(streamed.iter().all(|id| kept.contains(id)), "the stream's rows are the history's");
    assert!(history.iter().any(|r| r["k"] == "tool" && r["state"] == "ok"), "{history:#?}");
    bridge.stop();
    // Nothing left behind: the throwaway folder, and its transcript.
    std::fs::remove_file(&found.path).ok();
    if let Some(dir) = found.path.parent() {
        std::fs::remove_dir(dir).ok();
    }
    std::fs::remove_dir_all(&root).ok();
}
