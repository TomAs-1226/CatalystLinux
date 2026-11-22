//! Talking to the handheld: finding it (UDP, every network this computer
//! is on, and the USB cable), TLS 1.3 to it with its certificate pinned
//! once paired, one HTTP request a connection, and pairing.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use catalyst_companion_proto::{self as proto, pair};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use serde_json::{json, Value};

/// A handheld that answered.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub id: String,
    pub name: String,
    pub version: String,
    pub host: String,
    pub port: u16,
    pub usb: bool,
}

/// This computer's IPv4 addresses, one per network it is on.
fn local_addresses() -> Vec<Ipv4Addr> {
    let mut out: Vec<Ipv4Addr> = crate::win::local_ipv4().into_iter().filter(|v4| !v4.is_loopback() && !v4.is_link_local() && !v4.is_unspecified()).collect();
    out.sort();
    out.dedup();
    out
}

/// Asks every network for handhelds, and the USB cable's address, for
/// `wait`; each one that answers, once (by the cable if it answered there).
pub fn discover(wait: Duration) -> Vec<Found> {
    let mut sockets = Vec::new();
    // One socket on each network, so the broadcast goes out on each.
    for ip in local_addresses().into_iter().map(IpAddr::V4).chain([IpAddr::V4(Ipv4Addr::UNSPECIFIED)]) {
        if let Ok(s) = UdpSocket::bind((ip, 0)) {
            s.set_broadcast(true).ok();
            s.set_nonblocking(true).ok();
            sockets.push(s);
        }
    }
    let usb: SocketAddr = (proto::USB_ADDRESS.parse::<Ipv4Addr>().expect("an address"), proto::DISCOVERY_PORT).into();
    let targets: Vec<SocketAddr> = vec![(Ipv4Addr::BROADCAST, proto::DISCOVERY_PORT).into(), usb, (Ipv4Addr::LOCALHOST, proto::DISCOVERY_PORT).into()];
    let ask = |s: &UdpSocket| {
        for t in &targets {
            s.send_to(proto::DISCOVERY_ASK, t).ok();
        }
    };
    sockets.iter().for_each(ask);
    let started = Instant::now();
    let mut found: Vec<Found> = Vec::new();
    let mut buf = [0u8; 1024];
    let mut asked_again = false;
    while started.elapsed() < wait {
        // Once more halfway, for a packet lost on Wi-Fi.
        if !asked_again && started.elapsed() > wait / 2 {
            asked_again = true;
            sockets.iter().for_each(ask);
        }
        let mut any = false;
        for s in &sockets {
            while let Ok((n, from)) = s.recv_from(&mut buf) {
                any = true;
                let Ok(v) = serde_json::from_slice::<Value>(&buf[..n]) else { continue };
                if v["catalyst"] != 1 {
                    continue;
                }
                let f = Found {
                    id: v["id"].as_str().unwrap_or_default().to_owned(),
                    name: v["name"].as_str().unwrap_or("catalyst").to_owned(),
                    version: v["version"].as_str().unwrap_or_default().to_owned(),
                    host: from.ip().to_string(),
                    port: v["port"].as_u64().and_then(|p| u16::try_from(p).ok()).unwrap_or(proto::PORT),
                    usb: from == usb || from.ip().to_string() == proto::USB_ADDRESS,
                };
                match found.iter_mut().find(|x| x.id == f.id) {
                    Some(x) if f.usb && !x.usb => *x = f,
                    Some(_) => {}
                    None => found.push(f),
                }
            }
        }
        if !any {
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    found
}

// -------------------------------------------------------------------- TLS ---

/// Takes the handheld's certificate if it is the one pinned (any, while
/// pairing), checks the handshake is signed by it, and notes what it saw.
#[derive(Debug)]
struct Pinned {
    want: Option<[u8; 32]>,
    seen: Mutex<Option<[u8; 32]>>,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(&self, end_entity: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        let fp = proto::fingerprint(end_entity.as_ref());
        *self.seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(fp);
        match self.want {
            Some(want) if !proto::same(&want, &fp) => Err(rustls::Error::General("this is not the handheld this computer paired with".into())),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(&self, _: &[u8], _: &CertificateDer<'_>, _: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("TLS 1.3 only".into()))
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

type Tls = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

/// What went wrong, put plainly.
fn plainly(e: &std::io::Error) -> String {
    let text = e.to_string();
    if text.contains("not the handheld this computer paired with") {
        "the handheld answered with another certificate: pair again if it was reset".into()
    } else if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) {
        "the handheld stopped answering".into()
    } else if matches!(e.kind(), std::io::ErrorKind::ConnectionRefused) {
        "the handheld is not taking connections: is Computer companion on?".into()
    } else {
        format!("the connection failed ({text})")
    }
}

/// A TLS connection to `host:port`, handshake done; and the fingerprint seen.
fn connect(host: &str, port: u16, pin: Option<[u8; 32]>, read_for: Duration) -> Result<(Tls, [u8; 32]), String> {
    let addr = (host, port).to_socket_addrs().map_err(|_| format!("{host} is not an address"))?.next().ok_or("no address")?;
    let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(4)).map_err(|e| plainly(&e))?;
    tcp.set_read_timeout(Some(read_for)).ok();
    tcp.set_write_timeout(Some(Duration::from_secs(60))).ok();
    tcp.set_nodelay(true).ok();
    let mut provider = rustls::crypto::ring::default_provider();
    provider.cipher_suites = vec![rustls::crypto::ring::cipher_suite::TLS13_CHACHA20_POLY1305_SHA256, rustls::crypto::ring::cipher_suite::TLS13_AES_128_GCM_SHA256];
    let provider = Arc::new(provider);
    let verifier = Arc::new(Pinned { want: pin, seen: Mutex::new(None), provider: provider.clone() });
    let config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(verifier.clone())
        .with_no_client_auth();
    let name = ServerName::try_from("catalyst.local").expect("a name");
    let conn = rustls::ClientConnection::new(Arc::new(config), name).map_err(|e| e.to_string())?;
    let mut tls = rustls::StreamOwned::new(conn, tcp);
    while tls.conn.is_handshaking() {
        tls.conn.complete_io(&mut tls.sock).map_err(|e| plainly(&e))?;
    }
    let seen = verifier.seen.lock().unwrap_or_else(|e| e.into_inner()).ok_or("the handheld showed no certificate")?;
    Ok((tls, seen))
}

/// How a request's body is sent.
pub enum Send<'a> {
    None,
    Json(Value),
    Text(String),
    /// `len` bytes from a reader, with the bytes sent so far told as they go.
    Stream { len: u64, from: &'a mut dyn Read, progress: &'a mut dyn FnMut(u64) },
}

/// Where an answer's body goes.
pub enum Sink<'a> {
    Memory,
    /// To a writer, with the bytes so far and the whole length (if known).
    Writer { to: &'a mut dyn Write, progress: &'a mut dyn FnMut(u64, Option<u64>) },
}

pub struct Answer {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Answer {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    /// The JSON on success, or what the handheld said went wrong.
    pub fn ok(self) -> Result<Value, String> {
        if self.status == 200 {
            return Ok(self.json());
        }
        Err(self.json()["error"].as_str().map_or_else(|| format!("the handheld answered {}", self.status), str::to_owned))
    }
}

/// A paired handheld, as a request reaches it.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub host: String,
    pub port: u16,
    pub pin: [u8; 32],
    pub token: String,
}

fn exchange(tls: &mut Tls, method: &str, path: &str, token: Option<&str>, send: Send, into: Sink) -> Result<Answer, String> {
    let (kind, len) = match &send {
        Send::None => ("application/json", 0),
        Send::Json(v) => ("application/json", v.to_string().len() as u64),
        Send::Text(t) => ("text/plain; charset=utf-8", t.len() as u64),
        Send::Stream { len, .. } => ("application/octet-stream", *len),
    };
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let head = format!("{method} {path} HTTP/1.1\r\nHost: catalyst.local\r\nUser-Agent: catalyst-companion\r\n{auth}Content-Type: {kind}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n");
    let sent = (|| -> std::io::Result<()> {
        tls.write_all(head.as_bytes())?;
        match send {
            Send::None => {}
            Send::Json(v) => tls.write_all(v.to_string().as_bytes())?,
            Send::Text(t) => tls.write_all(t.as_bytes())?,
            Send::Stream { len, from, progress } => {
                let mut buf = vec![0u8; 64 * 1024];
                let mut done = 0u64;
                let mut from = from.take(len);
                loop {
                    let n = from.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    tls.write_all(&buf[..n])?;
                    done += n as u64;
                    progress(done);
                }
                if done != len {
                    return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the file changed while it was sent"));
                }
            }
        }
        tls.flush()
    })();
    // A refusal may come before the body was all taken: read it all the same.
    let mut reader = BufReader::new(tls);
    let mut line = String::new();
    if reader.read_line(&mut line).map_err(|e| plainly(&e))? == 0 {
        return Err(match sent {
            Err(e) => plainly(&e),
            Ok(()) => "the handheld closed the connection".into(),
        });
    }
    let status: u16 = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or("the handheld's answer was not HTTP")?;
    let mut length: Option<u64> = None;
    loop {
        line.clear();
        reader.read_line(&mut line).map_err(|e| plainly(&e))?;
        let l = line.trim();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                length = v.trim().parse().ok();
            }
        }
    }
    let mut body = Vec::new();
    match into {
        Sink::Writer { to, progress } if status == 200 => {
            let mut buf = vec![0u8; 64 * 1024];
            let mut got = 0u64;
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(n) => n,
                    // The end as some servers close; the length says whether it was all.
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => 0,
                    Err(e) => return Err(plainly(&e)),
                };
                if n == 0 {
                    break;
                }
                to.write_all(&buf[..n]).map_err(|e| format!("could not write it on this computer: {e}"))?;
                got += n as u64;
                progress(got, length);
            }
            if length.is_some_and(|l| l != got) {
                return Err("the download stopped part way".into());
            }
        }
        _ => {
            let most = length.unwrap_or(16 << 20).min(64 << 20);
            match (&mut reader).take(most).read_to_end(&mut body) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof && length.is_some_and(|l| l == body.len() as u64) => {}
                Err(e) => return Err(plainly(&e)),
            }
        }
    }
    let tls = reader.into_inner();
    tls.conn.send_close_notify();
    tls.flush().ok();
    Ok(Answer { status, body })
}

impl Target {
    /// A request, with a longer wait for one the handheld takes time over.
    pub fn call(&self, method: &str, path: &str, send: Send, into: Sink, patience: Duration) -> Result<Answer, String> {
        let (mut tls, _) = connect(&self.host, self.port, Some(self.pin), patience)?;
        exchange(&mut tls, method, path, Some(&self.token), send, into)
    }

    pub fn get(&self, path: &str) -> Result<Value, String> {
        self.call("GET", path, Send::None, Sink::Memory, Duration::from_secs(20))?.ok()
    }

    pub fn post(&self, path: &str, v: Value) -> Result<Value, String> {
        self.call("POST", path, Send::Json(v), Sink::Memory, Duration::from_secs(30))?.ok()
    }

    pub fn delete(&self, path: &str) -> Result<Value, String> {
        self.call("DELETE", path, Send::None, Sink::Memory, Duration::from_secs(20))?.ok()
    }
}

/// `a b/é` → `a%20b%2F%C3%A9`: for a query's values.
pub fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// What pairing gives: the handheld, and how to reach it from now on.
#[derive(Clone, Debug)]
pub struct Paired {
    pub id: String,
    pub name: String,
    pub version: String,
    pub target: Target,
}

/// Pairs with the handheld at `host:port` using the code on its screen.
pub fn pair(host: &str, port: u16, code: &str, computer: &str) -> Result<Paired, String> {
    let code = proto::clean_code(code).ok_or("the code is the six digits on the handheld's screen")?;
    let (mut tls, seen) = connect(host, port, None, Duration::from_secs(20))?;
    let (me, first) = pair::Computer::start(&code);
    let reply = exchange(&mut tls, "POST", "/v1/pair", None, Send::Json(json!({"name": computer, "message": proto::hex(&first)})), Sink::Memory)?.ok()?;
    let theirs = reply["message"].as_str().and_then(proto::unhex).ok_or("the handheld's answer was not one")?;
    let key = me.finish(&theirs)?;
    let proof = reply["proof"].as_str().and_then(proto::unhex).unwrap_or_default();
    if !proto::same(&proof, &pair::confirm(&key, pair::Side::Handheld, &seen)) {
        // A wrong code, or something between here and the handheld: either way, nothing more is sent.
        return Err("that code did not match: check it on the handheld and type it again".into());
    }
    // From here on, only this certificate.
    let (mut tls, _) = connect(host, port, Some(seen), Duration::from_secs(20))?;
    let ours = pair::confirm(&key, pair::Side::Computer, &seen);
    let done = exchange(&mut tls, "POST", "/v1/pair/confirm", None, Send::Json(json!({"session": reply["session"], "proof": proto::hex(&ours)})), Sink::Memory)?.ok()?;
    let token = done["token"].as_str().ok_or("the handheld gave no token")?.to_owned();
    Ok(Paired {
        id: done["id"].as_str().unwrap_or_default().to_owned(),
        name: done["name"].as_str().unwrap_or("catalyst").to_owned(),
        version: done["version"].as_str().unwrap_or_default().to_owned(),
        target: Target { host: host.to_owned(), port, pin: seen, token },
    })
}
