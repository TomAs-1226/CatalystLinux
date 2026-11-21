//! The Catalyst Agent: the read-only service on the Systemcore (HTTP, port
//! 9010) that reports on the machine itself: the robot program's state and
//! recent log, CPU, memory, CAN buses.
//!
//! The agent samples when asked, so this client only polls while something
//! is showing its data: create it when a screen opens, drop it on leaving.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::Value;

pub const PORT: u16 = 9010;
const POLL: Duration = Duration::from_secs(3);

#[derive(Default)]
struct Shared {
    snapshot: Mutex<Option<Value>>,
    status: Mutex<String>,
    generation: AtomicU64,
    stop: AtomicBool,
}

pub struct Agent {
    shared: Arc<Shared>,
}

impl Agent {
    /// Start polling `/api/system` on `host`.
    pub fn connect(host: &str) -> Agent {
        let shared = Arc::new(Shared::default());
        *shared.status.lock().unwrap() = "asking the agent".into();
        let (thread_shared, host) = (shared.clone(), host.to_owned());
        thread::Builder::new()
            .name("agent".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                while !thread_shared.stop.load(Ordering::Relaxed) {
                    match get(&host, "/api/system") {
                        Ok(snapshot) => {
                            *thread_shared.snapshot.lock().unwrap() = Some(snapshot);
                            *thread_shared.status.lock().unwrap() = "ok".into();
                        }
                        Err(why) => {
                            *thread_shared.snapshot.lock().unwrap() = None;
                            *thread_shared.status.lock().unwrap() = why;
                        }
                    }
                    thread_shared.generation.fetch_add(1, Ordering::Relaxed);
                    let mut waited = Duration::ZERO;
                    while waited < POLL && !thread_shared.stop.load(Ordering::Relaxed) {
                        // In deep standby, parked here: no wakeups until it wakes.
                        crate::deep::rest();
                        thread::sleep(Duration::from_millis(100));
                        waited += Duration::from_millis(100);
                    }
                }
            })
            .expect("spawn agent thread");
        Agent { shared }
    }

    /// A scripted agent to go with the demo robot.
    pub fn demo() -> Agent {
        let shared = Arc::new(Shared::default());
        *shared.snapshot.lock().unwrap() = Some(serde_json::json!({
            "identity": {"hostname": "systemcore", "os": "Systemcore OS beta 14", "agentVersion": "2.0.3", "uptimeSeconds": 5421},
            "cpu": {"loadAverage": [0.82, 0.74, 0.61]},
            "memory": {"totalBytes": 8_000_000_000u64, "usedBytes": 1_420_000_000u64},
            "robotProgram": {
                "unit": "robot.service", "state": "active", "subState": "running", "restarts": 1,
                "runningForSeconds": 1287, "memoryBytes": 412_000_000,
                "log": [
                    "2026-11-16T10:02:11+0000 systemcore robot[812]: ********** Robot program starting **********",
                    "2026-11-16T10:02:13+0000 systemcore robot[812]: Catalyst 2.0.0 on WPILib 2027.0.0-alpha-7",
                    "2026-11-16T10:02:13+0000 systemcore robot[812]: Preflight: Ready, 1 warning",
                    "2026-11-16T10:02:14+0000 systemcore robot[812]: [warn] CAN plan — can_s0 has 14 devices",
                    "2026-11-16T10:02:14+0000 systemcore robot[812]: ********** Robot program startup complete **********",
                    "2026-11-16T10:09:40+0000 systemcore robot[812]: Catalyst WARNING: [Intake] OverCurrent 38 A of 40",
                    "2026-11-16T10:09:42+0000 systemcore robot[812]: Catalyst WARNING: [Intake] OverCurrent cleared",
                    "2026-11-16T10:21:03+0000 systemcore robot[812]: Catalyst WARNING: [CAN] can_s0 utilization high"
                ]
            }
        }));
        *shared.status.lock().unwrap() = "ok".into();
        Agent { shared }
    }

    pub fn snapshot(&self) -> Option<Value> {
        self.shared.snapshot.lock().unwrap().clone()
    }

    /// "ok", or why there is no data.
    pub fn status(&self) -> String {
        self.shared.status.lock().unwrap().clone()
    }

    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Relaxed)
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

/// Fetch the log `name` from byte `offset` on, writing what arrives to `out`
/// and counting it in `progress`. Returns the bytes written; 0 when the robot
/// has nothing past `offset`. Resuming is what makes a pulled plug harmless.
pub fn download(host: &str, name: &str, offset: u64, out: &mut impl Write, progress: &AtomicU64) -> Result<u64, String> {
    download_from((host, PORT), host, name, offset, out, progress)
}

fn download_from(at: (&str, u16), host: &str, name: &str, offset: u64, out: &mut impl Write, progress: &AtomicU64) -> Result<u64, String> {
    let addr = at.to_socket_addrs().ok().and_then(|mut a| a.next()).ok_or("cannot resolve the robot")?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(1500)).map_err(|_| "no answer from the robot".to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok();
    let encoded: String = name
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"._-".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect();
    let range = if offset > 0 { format!("Range: bytes={offset}-\r\n") } else { String::new() };
    let request = format!("GET /api/logs/{encoded} HTTP/1.1\r\nHost: {host}\r\n{range}Connection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).map_err(|_| "agent hung up".to_string())?;

    let mut head = Vec::with_capacity(1024);
    let mut buf = [0u8; 16 * 1024];
    let (body_start, length, skip) = loop {
        let n = stream.read(&mut buf).map_err(|_| "agent timed out".to_string())?;
        if n == 0 {
            return Err("agent sent no reply".into());
        }
        head.extend_from_slice(&buf[..n]);
        if let Some(at) = head.windows(4).position(|w| w == b"\r\n\r\n") {
            let text = String::from_utf8_lossy(&head[..at]).to_ascii_lowercase();
            let header = |name: &str| text.lines().find_map(|l| l.strip_prefix(name)).map(str::trim);
            let status = text.split_whitespace().nth(1).unwrap_or("");
            let length = header("content-length:").and_then(|v| v.parse::<u64>().ok()).ok_or("agent sent no length")?;
            let skip = match status {
                // The part asked for: it must start where this copy ends.
                "206" => {
                    let start = header("content-range:").and_then(|v| v.strip_prefix("bytes ")?.split('-').next()?.parse::<u64>().ok());
                    if start != Some(offset) {
                        return Err("agent sent the wrong part of the log".into());
                    }
                    0
                }
                // The whole file, the range ignored: pass over what is already here.
                "200" => offset,
                // Nothing past the offset: the file is already whole here.
                "416" => return Ok(0),
                "404" => return Err("agent has no log routes, or the log is gone".into()),
                other => return Err(format!("agent said {other}")),
            };
            if skip > length {
                return Err("the log on the robot is shorter than the copy here".into());
            }
            break (at + 4, length, skip);
        }
        if head.len() > 64 * 1024 {
            return Err("agent reply was not http".into());
        }
    };
    // `seen` counts every byte of the body; only those past `skip` are kept.
    let (mut seen, mut written) = (0u64, 0u64);
    let mut put = |chunk: &[u8], seen: &mut u64, written: &mut u64| -> Result<(), String> {
        let chunk = &chunk[..chunk.len().min((length - *seen) as usize)];
        let drop = (skip.saturating_sub(*seen) as usize).min(chunk.len());
        *seen += chunk.len() as u64;
        let keep = &chunk[drop..];
        out.write_all(keep).map_err(|e| format!("cannot save the log: {e}"))?;
        *written += keep.len() as u64;
        progress.fetch_add(keep.len() as u64, Ordering::Relaxed);
        Ok(())
    };
    put(&head[body_start..], &mut seen, &mut written)?;
    while seen < length {
        let n = stream.read(&mut buf).map_err(|_| "link dropped mid-log".to_string())?;
        if n == 0 {
            break; // short: the next pull resumes from here
        }
        put(&buf[..n], &mut seen, &mut written)?;
    }
    Ok(written)
}

#[cfg(test)]
mod download_tests {
    use super::*;
    use std::net::TcpListener;

    /// Serve one canned reply on a local port, as an agent would.
    fn serve(reply: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0u8; 1024];
                stream.read(&mut request).ok();
                stream.write_all(&reply).ok();
            }
        });
        format!("127.0.0.1:{port}")
    }

    fn fetch(reply: &[u8], offset: u64) -> Result<Vec<u8>, String> {
        let host = serve(reply.to_vec());
        let (host, port) = host.split_once(':').unwrap();
        let mut out = Vec::new();
        download_from((host, port.parse().unwrap()), host, "x.wpilog", offset, &mut out, &AtomicU64::new(0))?;
        Ok(out)
    }

    #[test]
    fn a_resumed_pull_keeps_only_what_is_missing() {
        // An agent that honours the range: the rest of the file.
        assert_eq!(fetch(b"HTTP/1.1 206 Partial\r\nContent-Length: 4\r\nContent-Range: bytes 6-9/10\r\n\r\nWXYZ", 6).unwrap(), b"WXYZ");
        // One that ignores it and sends the whole file: the part already here is passed over.
        assert_eq!(fetch(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabcdefWXYZ", 6).unwrap(), b"WXYZ");
        // A part that starts somewhere else is refused rather than appended.
        assert!(fetch(b"HTTP/1.1 206 Partial\r\nContent-Length: 4\r\nContent-Range: bytes 0-3/10\r\n\r\nabcd", 6).is_err());
        // Nothing more: the copy here is whole.
        assert_eq!(fetch(b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\n\r\n", 10).unwrap(), b"");
    }
}

/// One HTTP GET returning JSON. The agent always sends a Content-Length.
pub fn get(host: &str, path: &str) -> Result<Value, String> {
    let addr = (host, PORT).to_socket_addrs().ok().and_then(|mut a| a.next()).ok_or("cannot resolve the robot")?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(1500)).map_err(|e| match e.kind() {
        std::io::ErrorKind::ConnectionRefused => "agent not installed on the robot".to_string(),
        _ => "no answer from the robot".to_string(),
    })?;
    // A full snapshot takes the agent a few hundred ms to sample.
    stream.set_read_timeout(Some(Duration::from_secs(6))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok();
    // One write, so the request leaves as one packet.
    let request = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nAccept: application/json\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).map_err(|_| "agent hung up".to_string())?;

    let mut raw = Vec::with_capacity(16 * 1024);
    let mut buf = [0u8; 8192];
    let (mut body_start, mut want) = (None, None);
    loop {
        let n = match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => return Err("agent timed out".into()),
        };
        raw.extend_from_slice(&buf[..n]);
        if raw.len() > 4 * 1024 * 1024 {
            return Err("agent reply too large".into());
        }
        if body_start.is_none() {
            if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&raw[..at]).to_ascii_lowercase();
                if !head.starts_with("http/1.1 200") && !head.starts_with("http/1.0 200") {
                    return Err(format!("agent said {}", head.lines().next().unwrap_or("").trim_start_matches("http/1.1 ")));
                }
                want = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok());
                body_start = Some(at + 4);
            }
        }
        if let (Some(start), Some(len)) = (body_start, want) {
            if raw.len() >= start + len {
                break;
            }
        }
    }
    let start = body_start.ok_or("agent sent no reply")?;
    serde_json::from_slice(&raw[start..]).map_err(|_| "agent reply was not json".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// Against a real catalyst-agent: `CATALYST_AGENT_HOST=10.58.5.2 cargo test live_agent -- --ignored`.
    #[test]
    #[ignore = "needs a running catalyst-agent"]
    fn live_agent() {
        let host = std::env::var("CATALYST_AGENT_HOST").expect("set CATALYST_AGENT_HOST");
        let snapshot = get(&host, "/api/system").unwrap();
        for key in ["identity", "cpu", "memory", "robotProgram", "can", "network"] {
            assert!(snapshot.get(key).is_some(), "snapshot has no {key}");
        }
        let program = &snapshot["robotProgram"];
        println!("agent {} · program {} · {} log lines", snapshot["identity"]["agentVersion"], program["state"], program["log"].as_array().map_or(0, Vec::len));
    }

    /// The reply shape the agent's `_send` produces: HTTP/1.1, JSON, Content-Length.
    #[test]
    fn reads_an_agent_reply() {
        let listener = TcpListener::bind(("127.0.0.1", PORT)).expect("port 9010 free for the test");
        let server = thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut req = Vec::new();
            let mut buf = [0u8; 512];
            while !req.ends_with(b"\r\n\r\n") {
                let n = s.read(&mut buf).unwrap();
                req.extend_from_slice(&buf[..n]);
            }
            assert!(String::from_utf8_lossy(&req).starts_with("GET /api/system HTTP/1.1\r\n"));
            let body = r#"{"robotProgram": {"state": "active", "log": ["a", "b"]}}"#;
            // Sent in two pieces, as a real socket may deliver it.
            write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n", body.len()).unwrap();
            write!(s, "Access-Control-Allow-Origin: *\r\n\r\n{body}").unwrap();
        });
        let reply = get("127.0.0.1", "/api/system").unwrap();
        assert_eq!(reply["robotProgram"]["state"], "active");
        assert_eq!(reply["robotProgram"]["log"][1], "b");
        server.join().unwrap();
    }
}
