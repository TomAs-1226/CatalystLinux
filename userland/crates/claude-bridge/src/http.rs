//! Just enough HTTP/1.1 for the bridge: a request read whole (its size
//! capped, its time limited), a reply with its length, or a stream of
//! server-sent events ended by closing. Behind Tailscale Funnel, whose
//! proxy passes a reply of unknown length on as it comes.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// The largest request body taken.
pub const BODY_MOST: usize = 512 * 1024;
const HEAD_MOST: usize = 16 * 1024;

pub struct Request {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
}

/// A request from `stream`, or None for anything that is not one.
pub fn read(stream: &TcpStream) -> Option<Request> {
    stream.set_read_timeout(Some(Duration::from_secs(20))).ok()?;
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    let mut head = 0usize;
    reader.read_line(&mut line).ok()?;
    head += line.len();
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();
    let mut length = 0usize;
    loop {
        line.clear();
        let n = reader.read_line(&mut line).ok()?;
        head += n;
        if n == 0 || head > HEAD_MOST {
            return None;
        }
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((name, value)) = l.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().ok()?;
            }
            if name.trim().eq_ignore_ascii_case("transfer-encoding") {
                // Not taken: the handheld's curl sends a length.
                return None;
            }
        }
    }
    if length > BODY_MOST {
        return None;
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).ok()?;
    Some(Request { method, path, body })
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        _ => "Error",
    }
}

/// A whole reply.
pub fn reply(mut stream: &TcpStream, status: u16, content_type: &str, body: &[u8]) {
    let head = format!("HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", reason(status), body.len());
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

pub fn text(stream: &TcpStream, status: u16, body: &str) {
    reply(stream, status, "text/plain; charset=utf-8", body.as_bytes());
}

/// The head of a stream of events; its body ends when the connection does.
pub fn events_head(mut stream: &TcpStream) -> bool {
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nX-Accel-Buffering: no\r\nConnection: close\r\n\r\n";
    stream.write_all(head.as_bytes()).and_then(|_| stream.flush()).is_ok()
}

/// One event (`data: <sealed>`), or a comment to keep the connection alive.
pub fn event(mut stream: &TcpStream, data: Option<&str>) -> bool {
    let line = match data {
        Some(d) => format!("data: {d}\n\n"),
        None => ": still here\n\n".to_owned(),
    };
    stream.write_all(line.as_bytes()).and_then(|_| stream.flush()).is_ok()
}

/// A POST of JSON to the bridge on this computer, its JSON answer back
/// (the permission tool's way in), waiting up to `wait`.
pub fn post_local(port: u16, path: &str, body: &serde_json::Value, wait: Duration) -> Result<serde_json::Value, String> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).map_err(|e| format!("the bridge is not running: {e}"))?;
    s.set_read_timeout(Some(wait)).ok();
    let body = body.to_string();
    let req = format!("POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut answer = Vec::new();
    s.read_to_end(&mut answer).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&answer);
    let (head, body) = text.split_once("\r\n\r\n").ok_or("no answer")?;
    if !head.starts_with("HTTP/1.1 200") {
        return Err(head.lines().next().unwrap_or_default().to_owned());
    }
    serde_json::from_str(body).map_err(|e| e.to_string())
}
