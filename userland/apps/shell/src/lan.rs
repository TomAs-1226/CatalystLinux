//! What the pages the handheld serves on the local network share: the setup
//! page for a computer (remote.rs) and the phone gallery (phonegallery/).
//! Each listens only while its screen shows, for a few minutes at most.
//!
//! Here: the handheld's address on the network, a port (80, so the address
//! is just the number, else 8080), random codes and tokens from the
//! kernel's generator, and one request read with limits, so a client that
//! sends a byte now and then cannot hold a thread.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The handheld's address on whichever network it is on, looking at the
/// interfaces in `order` (the setup page prefers the cable, the phone
/// gallery the Wi-Fi a phone is on). Screenshots name one of their own
/// (`CATALYST_SETUP_ADDRESS`), not the computer's.
pub fn address(order: &[&str]) -> Option<String> {
    address_on(order).or_else(routed_address)
}

/// As [`address`], but only on the interfaces named: nothing else is a
/// network a phone could be on (the USB cable's is the computer's).
pub fn address_on(order: &[&str]) -> Option<String> {
    if let Ok(stand_in) = std::env::var("CATALYST_SETUP_ADDRESS") {
        return (!stand_in.is_empty()).then_some(stand_in);
    }
    order.iter().find_map(|iface| crate::sys::address(iface).filter(|a| a.contains('.')))
}

/// The address this computer would reach others from. Connecting a UDP
/// socket sends nothing; it only picks the route.
pub fn routed_address() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

/// Listen on every interface: port 80 where it can be had, else 8080. Not
/// blocking: the loop that takes connections looks in on its stop signal.
pub fn bind() -> Option<(TcpListener, u16)> {
    let (listener, port) = TcpListener::bind(("0.0.0.0", 80)).map(|l| (l, 80)).or_else(|_| TcpListener::bind(("0.0.0.0", 8080)).map(|l| (l, 8080))).ok()?;
    listener.set_nonblocking(true).ok()?;
    Some((listener, port))
}

/// `http://10.0.0.5`, with the port only when it is not 80.
pub fn url(ip: &str, port: u16) -> String {
    if port == 80 {
        format!("http://{ip}")
    } else {
        format!("http://{ip}:{port}")
    }
}

/// Bytes from the kernel's random generator (the system's on a computer);
/// should it ever fail, the process's own randomly seeded hashing keys.
pub fn random_bytes(out: &mut [u8]) {
    if getrandom::fill(out).is_ok() {
        return;
    }
    use std::hash::{BuildHasher, Hasher};
    for chunk in out.chunks_mut(8) {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        chunk.copy_from_slice(&h.finish().to_le_bytes()[..chunk.len()]);
    }
}

/// Six digits, to be typed.
pub fn one_time_code() -> String {
    let mut b = [0u8; 8];
    random_bytes(&mut b);
    format!("{:06}", u64::from_le_bytes(b) % 1_000_000)
}

/// Two strings the same, in a time that does not say how much of them was.
pub fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// How long a connection may wait and how much it may send.
pub struct Limits {
    /// Before its request starts: a browser opens a spare connection or two
    /// beside the one it uses, for its next request.
    pub idle: Duration,
    /// Once it has started, the whole request.
    pub request_in: Duration,
    /// The most a body may be.
    pub body_most: usize,
    /// More than that is read and passed over up to this much, so the
    /// client is told why rather than finding the connection gone.
    pub passed_over_most: usize,
}

/// A request as it arrived.
#[derive(Clone, Debug, Default)]
pub struct Request {
    pub method: String,
    /// The path, without its query.
    pub path: String,
    /// What follows `?`, as sent.
    pub query: String,
    /// Its Content-Type (a form's: urlencoded, or multipart with a file).
    pub kind: String,
    /// The request line and headers as sent, for the headers not kept above.
    pub head: String,
    pub body: String,
    /// More than the body limit was sent (and passed over).
    pub too_large: bool,
}

impl Request {
    /// A header's value, by its name in any case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().skip(1).find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim()))
    }

    /// A field of the query, decoded.
    pub fn query_field(&self, name: &str) -> Option<String> {
        parse_form(&self.query).into_iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }
}

/// One request: method, path, headers and body, read within `limits`. A
/// connection may wait quietly for a while before its request starts (a
/// browser's spare one), checking now and then that the page is still
/// offered; once a request has started it must arrive whole in a few
/// seconds, so a client sending a byte now and then cannot hold a thread.
pub fn read_request(mut stream: &TcpStream, stop: &AtomicBool, limits: &Limits) -> Option<Request> {
    stream.set_nonblocking(false).ok();
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    // The first byte: up to `idle`, a second at a time.
    let idle_until = Instant::now() + limits.idle;
    stream.set_read_timeout(Some(Duration::from_secs(1))).ok();
    let first = loop {
        match stream.read(&mut buf) {
            Ok(0) => return None,
            Ok(n) => break n,
            // Nothing yet: wait on, unless the page has closed or it has
            // waited long enough.
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if Instant::now() > idle_until || stop.load(Ordering::Relaxed) {
                    return None;
                }
            }
            Err(_) => return None,
        }
    };
    data.extend_from_slice(&buf[..first]);
    let deadline = Instant::now() + limits.request_in;
    let read = |stream: &mut &TcpStream, buf: &mut [u8]| -> Option<usize> {
        let left = deadline.checked_duration_since(Instant::now())?;
        stream.set_read_timeout(Some(left.max(Duration::from_millis(1)))).ok();
        stream.read(buf).ok()
    };
    let head_end = loop {
        if let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
        if data.len() > 16 * 1024 {
            return None;
        }
        let n = read(&mut stream, &mut buf)?;
        if n == 0 {
            return None;
        }
        data.extend_from_slice(&buf[..n]);
    };
    let head = String::from_utf8_lossy(&data[..head_end]).into_owned();
    let mut first = head.lines().next()?.split_whitespace();
    let method = first.next()?.to_owned();
    let target = first.next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let (path, query) = (path.to_owned(), query.to_owned());
    let mut request = Request { method, path, query, head, ..Request::default() };
    let length = request.header("content-length").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    request.kind = request.header("content-type").unwrap_or_default().to_owned();
    if length > limits.passed_over_most {
        return None;
    }
    // Too much: read to its end and let go, to say so.
    if length > limits.body_most {
        let mut have = data.len() - head_end;
        while have < length {
            match read(&mut stream, &mut buf) {
                Some(n) if n > 0 => have += n,
                _ => break,
            }
        }
        request.too_large = true;
        return Some(request);
    }
    while data.len() < head_end + length {
        let n = read(&mut stream, &mut buf)?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
    }
    request.body = String::from_utf8_lossy(&data[head_end..(head_end + length).min(data.len())]).into_owned();
    Some(request)
}

/// A whole reply in one go: text, never kept by the browser, closed after.
pub fn respond(mut stream: TcpStream, (status, kind, body): (&str, &str, String)) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).ok();
    stream.write_all(body.as_bytes()).ok();
}

/// `application/x-www-form-urlencoded` (a form, or a query) into name and
/// value pairs.
pub fn parse_form(body: &str) -> Vec<(String, String)> {
    let decode = |s: &str| {
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'+' => out.push(b' '),
                b'%' if i + 2 < bytes.len() => {
                    match u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"), 16) {
                        Ok(b) => {
                            out.push(b);
                            i += 2;
                        }
                        Err(_) => out.push(b'%'),
                    }
                }
                b => out.push(b),
            }
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    };
    body.split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(k), decode(v))
        })
        .collect()
}

/// A path's part decoded strictly: every `%` followed by two hex digits,
/// and UTF-8 once decoded. Anything else is no name at all.
pub fn decode_path(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_tokens_are_random_and_compared_whole() {
        let a = one_time_code();
        assert!(a.len() == 6 && a.chars().all(|c| c.is_ascii_digit()));
        let (mut x, mut y) = ([0u8; 16], [0u8; 16]);
        random_bytes(&mut x);
        random_bytes(&mut y);
        assert_ne!(x, y);
        assert!(same("ABC", "ABC") && !same("ABC", "ABD") && !same("ABC", "AB"));
    }

    #[test]
    fn a_path_is_decoded_strictly() {
        assert_eq!(decode_path("IMG_1%20a.jpg").as_deref(), Some("IMG_1 a.jpg"));
        assert_eq!(decode_path("%2e%2e%2Fetc").as_deref(), Some("../etc"), "decoded; the caller refuses it");
        assert_eq!(decode_path("bad%zz"), None);
        assert_eq!(decode_path("cut%2"), None);
        assert_eq!(decode_path("%ff"), None, "not UTF-8");
    }
}
