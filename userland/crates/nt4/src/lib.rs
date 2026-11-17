//! A NetworkTables 4.1 client.
//!
//! NT4 is a WebSocket (port 5810, path `/nt/<client name>`): JSON text frames
//! announce topics and carry subscriptions, MessagePack binary frames carry
//! values as `[topic id, timestamp µs, type id, value]`.
//!
//! This client is read-only by design (it subscribes, it never publishes, as
//! Catalyst's own companions do). One background thread owns the socket,
//! reconnects on its own, and keeps a table of the latest value per topic
//! that any thread can read.

use std::collections::BTreeMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tungstenite::client::IntoClientRequest;
use tungstenite::http::HeaderValue;
use tungstenite::{Message, WebSocket};

pub const PORT: u16 = 5810;
const SUBPROTOCOLS: &str = "v4.1.networktables.first.wpi.edu, networktables.first.wpi.edu";

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Double(f64),
    Int(i64),
    Float(f32),
    String(String),
    /// Raw bytes: also structs, protobuf and msgpack payloads.
    Raw(Vec<u8>),
    BoolArray(Vec<bool>),
    DoubleArray(Vec<f64>),
    IntArray(Vec<i64>),
    FloatArray(Vec<f32>),
    StringArray(Vec<String>),
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Double(v) => Some(*v),
            Value::Float(v) => Some(*v as f64),
            Value::Int(v) => Some(*v as f64),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        if let Value::Bool(v) = self { Some(*v) } else { None }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(v) => Some(*v),
            Value::Double(v) => Some(*v as i64),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        if let Value::String(v) = self { Some(v) } else { None }
    }
    pub fn as_strings(&self) -> Option<&[String]> {
        if let Value::StringArray(v) = self { Some(v) } else { None }
    }
    pub fn as_f64s(&self) -> Option<&[f64]> {
        if let Value::DoubleArray(v) = self { Some(v) } else { None }
    }

    /// A short rendering for a value browser.
    pub fn display(&self) -> String {
        fn list<T>(v: &[T], f: impl Fn(&T) -> String) -> String {
            let shown: Vec<String> = v.iter().take(6).map(f).collect();
            let more = if v.len() > 6 { format!(", … {}", v.len()) } else { String::new() };
            format!("[{}{}]", shown.join(", "), more)
        }
        match self {
            Value::Bool(v) => v.to_string(),
            Value::Double(v) => trim_float(*v),
            Value::Float(v) => trim_float(*v as f64),
            Value::Int(v) => v.to_string(),
            Value::String(v) => v.clone(),
            Value::Raw(v) => format!("{} bytes", v.len()),
            Value::BoolArray(v) => list(v, |b| b.to_string()),
            Value::DoubleArray(v) => list(v, |d| trim_float(*d)),
            Value::FloatArray(v) => list(v, |d| trim_float(*d as f64)),
            Value::IntArray(v) => list(v, |i| i.to_string()),
            Value::StringArray(v) => list(v, |s| s.clone()),
        }
    }
}

fn trim_float(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e9 {
        format!("{v:.0}")
    } else if v.abs() >= 100.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.3}")
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    /// The NT type string: `double`, `string[]`, `struct:Pose2d`, …
    pub type_name: String,
    /// Absent until the first value arrives.
    pub value: Option<Value>,
    /// Server time of the value, microseconds.
    pub timestamp: i64,
}

#[derive(Default)]
struct Shared {
    table: Mutex<BTreeMap<String, Entry>>,
    connected: AtomicBool,
    generation: AtomicU64,
    stop: AtomicBool,
    status: Mutex<String>,
    /// Round trip to the server, microseconds; 0 until measured.
    rtt_us: AtomicU64,
}

/// A handle to the client thread. Dropping it stops the thread.
pub struct Client {
    shared: Arc<Shared>,
    address: String,
}

impl Client {
    /// Start connecting to `address` (`host` or `host:port`), subscribing to
    /// every topic under each prefix. Returns at once; the thread keeps
    /// retrying until dropped.
    pub fn connect(address: &str, client_name: &str, prefixes: &[&str]) -> Client {
        let shared = Arc::new(Shared::default());
        let address = if address.contains(':') { address.to_owned() } else { format!("{address}:{PORT}") };
        *shared.status.lock().unwrap() = "connecting".into();
        let thread_shared = shared.clone();
        let (addr, name) = (address.clone(), client_name.to_owned());
        let prefixes: Vec<String> = prefixes.iter().map(|s| s.to_string()).collect();
        thread::Builder::new()
            .name("nt4".into())
            .stack_size(256 * 1024)
            .spawn(move || run(thread_shared, addr, name, prefixes))
            .expect("spawn nt4 thread");
        Client { shared, address }
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn connected(&self) -> bool {
        self.shared.connected.load(Ordering::Relaxed)
    }

    /// Why the client is not connected, or "connected".
    pub fn status(&self) -> String {
        self.shared.status.lock().unwrap().clone()
    }

    /// Changes whenever the table or the connection state does; compare it
    /// with the last value seen to know whether to redraw.
    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Relaxed)
    }

    pub fn rtt_ms(&self) -> Option<f32> {
        match self.shared.rtt_us.load(Ordering::Relaxed) {
            0 => None,
            us => Some(us as f32 / 1000.0),
        }
    }

    /// The latest value of one topic. Absent means absent: Catalyst never
    /// publishes a placeholder for an unknown value.
    pub fn get(&self, topic: &str) -> Option<Value> {
        self.shared.table.lock().unwrap().get(topic).and_then(|e| e.value.clone())
    }

    pub fn f64(&self, topic: &str) -> Option<f64> {
        self.get(topic).and_then(|v| v.as_f64())
    }
    pub fn bool(&self, topic: &str) -> Option<bool> {
        self.get(topic).and_then(|v| v.as_bool())
    }
    pub fn i64(&self, topic: &str) -> Option<i64> {
        self.get(topic).and_then(|v| v.as_i64())
    }
    pub fn string(&self, topic: &str) -> Option<String> {
        self.get(topic).and_then(|v| v.as_str().map(str::to_owned))
    }
    pub fn strings(&self, topic: &str) -> Option<Vec<String>> {
        self.get(topic).and_then(|v| v.as_strings().map(<[String]>::to_vec))
    }

    /// Every topic whose name starts with `prefix`, in name order.
    pub fn with_prefix(&self, prefix: &str) -> Vec<(String, Entry)> {
        let table = self.shared.table.lock().unwrap();
        table
            .range(prefix.to_owned()..)
            .take_while(|(k, _)| k.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    pub fn topic_count(&self) -> usize {
        self.shared.table.lock().unwrap().len()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

fn set_status(shared: &Shared, status: impl Into<String>) {
    *shared.status.lock().unwrap() = status.into();
    shared.generation.fetch_add(1, Ordering::Relaxed);
}

fn run(shared: Arc<Shared>, address: String, name: String, prefixes: Vec<String>) {
    let epoch = Instant::now();
    while !shared.stop.load(Ordering::Relaxed) {
        match session(&shared, &address, &name, &prefixes, epoch) {
            Ok(()) => return,
            Err(why) => {
                shared.connected.store(false, Ordering::Relaxed);
                shared.table.lock().unwrap().clear();
                shared.rtt_us.store(0, Ordering::Relaxed);
                set_status(&shared, why);
            }
        }
        // Retry, but stay quick to stop.
        for _ in 0..10 {
            if shared.stop.load(Ordering::Relaxed) {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

/// One connection, from handshake until it drops. `Ok` only when asked to stop.
fn session(shared: &Shared, address: &str, name: &str, prefixes: &[String], epoch: Instant) -> Result<(), String> {
    let socket_addr = address
        .to_socket_addrs()
        .map_err(|_| format!("cannot resolve {address}"))?
        .next()
        .ok_or_else(|| format!("cannot resolve {address}"))?;
    let stream = TcpStream::connect_timeout(&socket_addr, Duration::from_millis(1500)).map_err(|e| match e.kind() {
        std::io::ErrorKind::TimedOut => "no answer".to_string(),
        std::io::ErrorKind::ConnectionRefused => "robot code not running".to_string(),
        _ => "unreachable".to_string(),
    })?;
    stream.set_nodelay(true).ok();
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(3))).ok();

    let mut request = format!("ws://{address}/nt/{name}").into_client_request().map_err(|e| e.to_string())?;
    request.headers_mut().insert("Sec-WebSocket-Protocol", HeaderValue::from_static(SUBPROTOCOLS));
    let (mut ws, _) = tungstenite::client(request, stream).map_err(|_| "handshake refused".to_string())?;
    ws.get_ref().set_read_timeout(Some(Duration::from_millis(100))).ok();

    let subscribe = serde_json::json!([{
        "method": "subscribe",
        "params": { "topics": prefixes, "subuid": 1, "options": { "prefix": true, "periodic": 0.1 } }
    }]);
    ws.send(Message::text(subscribe.to_string())).map_err(|e| e.to_string())?;

    shared.connected.store(true, Ordering::Relaxed);
    set_status(shared, "connected");

    let mut names: BTreeMap<i64, String> = BTreeMap::new();
    let mut last_sync = Instant::now() - Duration::from_secs(10);
    let mut last_heard = Instant::now();
    loop {
        if shared.stop.load(Ordering::Relaxed) {
            ws.close(None).ok();
            return Ok(());
        }
        if last_sync.elapsed() > Duration::from_secs(2) {
            last_sync = Instant::now();
            send_time_sync(&mut ws, epoch)?;
        }
        match ws.read() {
            Ok(Message::Text(text)) => {
                last_heard = Instant::now();
                if handle_text(shared, &mut names, text.as_str()) {
                    shared.generation.fetch_add(1, Ordering::Relaxed);
                }
            }
            Ok(Message::Binary(bytes)) => {
                last_heard = Instant::now();
                if handle_binary(shared, &names, &bytes, epoch) {
                    shared.generation.fetch_add(1, Ordering::Relaxed);
                }
            }
            Ok(Message::Close(_)) => return Err("closed by robot".into()),
            Ok(_) => last_heard = Instant::now(),
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) =>
            {
                // The time sync every 2 s is answered, so silence means the link is gone.
                if last_heard.elapsed() > Duration::from_secs(6) {
                    return Err("link lost".into());
                }
            }
            Err(_) => return Err("link lost".into()),
        }
    }
}

fn send_time_sync(ws: &mut WebSocket<TcpStream>, epoch: Instant) -> Result<(), String> {
    let now = epoch.elapsed().as_micros() as i64;
    let frame = rmpv::Value::Array(vec![(-1).into(), 0.into(), 2.into(), now.into()]);
    let mut buf = Vec::with_capacity(16);
    rmpv::encode::write_value(&mut buf, &frame).map_err(|e| e.to_string())?;
    ws.send(Message::binary(buf)).map_err(|_| "link lost".to_string())
}

/// Announcements and removals. Returns whether the table changed.
fn handle_text(shared: &Shared, names: &mut BTreeMap<i64, String>, text: &str) -> bool {
    let Ok(serde_json::Value::Array(messages)) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    let mut changed = false;
    for message in messages {
        let params = &message["params"];
        let Some(name) = params["name"].as_str() else { continue };
        match message["method"].as_str() {
            Some("announce") => {
                let Some(id) = params["id"].as_i64() else { continue };
                names.insert(id, name.to_owned());
                let type_name = params["type"].as_str().unwrap_or("").to_owned();
                shared
                    .table
                    .lock()
                    .unwrap()
                    .entry(name.to_owned())
                    .or_insert(Entry { type_name, value: None, timestamp: 0 });
                changed = true;
            }
            Some("unannounce") => {
                names.retain(|_, n| n != name);
                changed |= shared.table.lock().unwrap().remove(name).is_some();
            }
            _ => {}
        }
    }
    changed
}

/// Value frames; several may share one WebSocket message.
fn handle_binary(shared: &Shared, names: &BTreeMap<i64, String>, bytes: &[u8], epoch: Instant) -> bool {
    let mut cursor = bytes;
    let mut changed = false;
    let mut table = None;
    while !cursor.is_empty() {
        let Ok(rmpv::Value::Array(frame)) = rmpv::decode::read_value(&mut cursor) else { break };
        if frame.len() != 4 {
            continue;
        }
        let (Some(id), Some(timestamp), Some(type_id)) = (frame[0].as_i64(), frame[1].as_i64(), frame[2].as_u64()) else {
            continue;
        };
        if id == -1 {
            // Time sync reply: the value is the client time we sent.
            if let Some(sent) = frame[3].as_i64() {
                let rtt = (epoch.elapsed().as_micros() as i64 - sent).max(1);
                shared.rtt_us.store(rtt as u64, Ordering::Relaxed);
            }
            continue;
        }
        let Some(name) = names.get(&id) else { continue };
        let Some(value) = decode(type_id, &frame[3]) else { continue };
        let table = table.get_or_insert_with(|| shared.table.lock().unwrap());
        if let Some(entry) = table.get_mut(name) {
            if entry.value.as_ref() != Some(&value) {
                changed = true;
            }
            entry.value = Some(value);
            entry.timestamp = timestamp;
        }
    }
    changed
}

fn decode(type_id: u64, v: &rmpv::Value) -> Option<Value> {
    Some(match type_id {
        0 => Value::Bool(v.as_bool()?),
        1 => Value::Double(v.as_f64()?),
        2 => Value::Int(v.as_i64()?),
        3 => Value::Float(v.as_f64()? as f32),
        4 => Value::String(v.as_str()?.to_owned()),
        5 => Value::Raw(v.as_slice()?.to_vec()),
        16 => Value::BoolArray(v.as_array()?.iter().map(|x| x.as_bool()).collect::<Option<_>>()?),
        17 => Value::DoubleArray(v.as_array()?.iter().map(|x| x.as_f64()).collect::<Option<_>>()?),
        18 => Value::IntArray(v.as_array()?.iter().map(|x| x.as_i64()).collect::<Option<_>>()?),
        19 => Value::FloatArray(v.as_array()?.iter().map(|x| x.as_f64().map(|f| f as f32)).collect::<Option<_>>()?),
        20 => Value::StringArray(v.as_array()?.iter().map(|x| x.as_str().map(str::to_owned)).collect::<Option<_>>()?),
        _ => return None,
    })
}
