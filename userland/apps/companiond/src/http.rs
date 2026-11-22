//! Just enough HTTP/1.1: one request a connection, its body by
//! Content-Length, and the answer (a stream that ends with the connection
//! when its length is not known).

use std::io::{self, BufRead, Read, Write};
use std::path::PathBuf;

use serde_json::Value;

/// The most a request's line and headers may take.
const HEAD_MOST: usize = 16 * 1024;

#[derive(Clone, Debug, Default)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub length: u64,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn query(&self, name: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// The token it carries.
    pub fn token(&self) -> Option<&str> {
        self.header("authorization")?.strip_prefix("Bearer ").map(str::trim).filter(|t| !t.is_empty())
    }
}

/// `%41+b` → `A b`.
pub fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                match std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `a b&c` → `a%20b%26c`: for a form's fields.
pub fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// The request's line and headers, or `None` if the connection closed first.
pub fn read_request(r: &mut dyn BufRead) -> io::Result<Option<Request>> {
    let mut head = Vec::new();
    loop {
        let before = head.len();
        let n = r.take((HEAD_MOST - head.len().min(HEAD_MOST)) as u64).read_until(b'\n', &mut head)?;
        if n == 0 {
            return if head.is_empty() { Ok(None) } else { Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the request stopped")) };
        }
        if head.len() >= HEAD_MOST {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "the request's headers are too long"));
        }
        let line = &head[before..];
        if line == b"\r\n" || line == b"\n" {
            break;
        }
    }
    let text = String::from_utf8_lossy(&head);
    let mut lines = text.lines();
    let first = lines.next().unwrap_or_default();
    let mut parts = first.split_whitespace();
    let (method, target) = (parts.next().unwrap_or_default().to_owned(), parts.next().unwrap_or_default());
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let query = query.split('&').filter(|p| !p.is_empty()).map(|p| p.split_once('=').unwrap_or((p, ""))).map(|(k, v)| (decode(k), decode(v))).collect();
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned())).collect();
    let length = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    if method.is_empty() || !path.starts_with('/') {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not an HTTP request"));
    }
    Ok(Some(Request { method, path: decode(path), query, headers, length }))
}

/// Writes the body of an answer whose length is not known before.
pub type Writer = Box<dyn FnOnce(&mut dyn Write) -> io::Result<()> + Send>;

pub enum Body {
    Json(Value),
    File { path: PathBuf, len: u64, name: String },
    /// Bytes to the end, `len` of them when it is known.
    Stream { len: Option<u64>, kind: &'static str, write: Writer },
}

pub struct Response {
    pub status: u16,
    pub body: Body,
    /// Done once the answer is sent (a restart, say).
    pub after: Option<Box<dyn FnOnce() + Send>>,
}

impl Response {
    pub fn json(status: u16, v: Value) -> Response {
        Response { status, body: Body::Json(v), after: None }
    }

    pub fn error(status: u16, why: impl Into<String>) -> Response {
        Response::json(status, serde_json::json!({ "error": why.into() }))
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        410 => "Gone",
        413 => "Payload Too Large",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        507 => "Insufficient Storage",
        _ => "Error",
    }
}

/// A name as a Content-Disposition header can carry it.
fn attachment(name: &str) -> String {
    let plain: String = name.chars().map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' { c } else { '_' }).collect();
    format!("attachment; filename=\"{plain}\"; filename*=UTF-8''{}", encode(name))
}

pub fn write_response(w: &mut dyn Write, response: Response) -> io::Result<Option<Box<dyn FnOnce() + Send>>> {
    let status = response.status;
    let head = |kind: &str, len: Option<u64>, extra: &str| {
        let len = len.map_or(String::new(), |n| format!("Content-Length: {n}\r\n"));
        format!("HTTP/1.1 {status} {}\r\nContent-Type: {kind}\r\n{len}{extra}Cache-Control: no-store\r\nConnection: close\r\n\r\n", reason(status))
    };
    match response.body {
        Body::Json(v) => {
            let body = v.to_string();
            w.write_all(head("application/json", Some(body.len() as u64), "").as_bytes())?;
            w.write_all(body.as_bytes())?;
        }
        Body::File { path, len, name } => {
            let file = std::fs::File::open(&path)?;
            w.write_all(head("application/octet-stream", Some(len), &format!("Content-Disposition: {}\r\n", attachment(&name))).as_bytes())?;
            io::copy(&mut file.take(len), w)?;
        }
        Body::Stream { len, kind, write } => {
            w.write_all(head(kind, len, "").as_bytes())?;
            write(w)?;
        }
    }
    w.flush()?;
    Ok(response.after)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_reads_with_its_query_and_headers() {
        let raw = b"PUT /v1/music?name=My%20Song.mp3&x=a+b HTTP/1.1\r\nHost: h\r\nAuthorization: Bearer abc\r\nContent-Length: 5\r\n\r\nhello";
        let mut r = &raw[..];
        let req = read_request(&mut r).unwrap().unwrap();
        assert_eq!((req.method.as_str(), req.path.as_str(), req.length), ("PUT", "/v1/music", 5));
        assert_eq!(req.query("name"), Some("My Song.mp3"));
        assert_eq!(req.query("x"), Some("a b"));
        assert_eq!(req.token(), Some("abc"));
        assert_eq!(r, b"hello", "the body is left to read");
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode(&encode("a b&c=d/é")), "a b&c=d/é");
    }

    #[test]
    fn headers_past_the_limit_are_refused() {
        let raw = format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(HEAD_MOST));
        assert!(read_request(&mut raw.as_bytes()).is_err());
        assert!(read_request(&mut &b""[..]).unwrap().is_none());
    }
}
