//! Wi-Fi: scanning, joining and forgetting networks, through the system's
//! `wpa_supplicant` (driven with `wpa_cli`).
//!
//! Everything slow happens on a worker thread that lives only while a Wi-Fi
//! screen is open; the screen reads a snapshot. Where there is no
//! `wpa_cli` (a development PC) a scripted set of networks stands in, so the
//! screens can be built and shown.

use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const WPA_CLI: &str = "/usr/sbin/wpa_cli";
const TOGGLE: &str = "/usr/sbin/catalyst-wifi-toggle";
const ENABLED_FLAG: &str = "/data/catalyst/wifi-enabled";

#[derive(Clone, Debug, PartialEq)]
pub struct Network {
    pub ssid: String,
    /// Signal in dBm; nearer zero is stronger.
    pub signal: i32,
    pub secured: bool,
    /// Its id among the saved networks, if its password is kept.
    pub saved: Option<u32>,
    pub current: bool,
}

impl Network {
    /// Signal as 0–4 bars.
    pub fn bars(&self) -> usize {
        match self.signal {
            s if s >= -55 => 4,
            s if s >= -65 => 3,
            s if s >= -75 => 2,
            s if s >= -85 => 1,
            _ => 0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub enabled: bool,
    pub networks: Vec<Network>,
    /// A few words on what Wi-Fi is doing.
    pub status: String,
}

impl State {
    /// Joined, with an address: the network can be used ("ssid · 10.58.5.23").
    pub fn online(&self) -> bool {
        self.enabled && self.status.rsplit(" · ").next().is_some_and(|a| a.parse::<std::net::Ipv4Addr>().is_ok())
    }
}

enum Cmd {
    Enable(bool),
    Join { ssid: String, password: Option<String> },
    Forget(u32),
    Scan,
}

pub struct Wifi {
    shared: Arc<Shared>,
    tx: Sender<Cmd>,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    generation: AtomicU64,
    stop: AtomicBool,
}

impl Wifi {
    pub fn open() -> Wifi {
        let shared = Arc::new(Shared::default());
        let (tx, rx) = mpsc::channel();
        let thread_shared = shared.clone();
        let real = std::path::Path::new(WPA_CLI).exists();
        thread::Builder::new()
            .name("wifi".into())
            .stack_size(256 * 1024)
            .spawn(move || if real { run(&thread_shared, &rx) } else { run_demo(&thread_shared, &rx) })
            .expect("spawn wifi thread");
        Wifi { shared, tx }
    }

    pub fn state(&self) -> State {
        self.shared.state.lock().unwrap().clone()
    }

    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Relaxed)
    }

    pub fn enable(&self, on: bool) {
        self.tx.send(Cmd::Enable(on)).ok();
    }

    /// Join a network; `password` is `None` for an open one or one already saved.
    pub fn join(&self, ssid: &str, password: Option<String>) {
        self.tx.send(Cmd::Join { ssid: ssid.to_owned(), password }).ok();
    }

    pub fn forget(&self, id: u32) {
        self.tx.send(Cmd::Forget(id)).ok();
    }

    pub fn scan(&self) {
        self.tx.send(Cmd::Scan).ok();
    }
}

impl Drop for Wifi {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

fn publish(shared: &Shared, state: State) {
    let mut held = shared.state.lock().unwrap();
    if *held != state {
        *held = state;
        shared.generation.fetch_add(1, Ordering::Relaxed);
    }
}

// ------------------------------------------------------------- parsing ---

/// wpa_supplicant prints an SSID's unusual bytes as `\xNN` and escapes
/// quotes and backslashes; turn that back into the name.
pub fn unescape(ssid: &str) -> String {
    let mut bytes = Vec::with_capacity(ssid.len());
    let raw = ssid.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\\' && i + 1 < raw.len() {
            match raw[i + 1] {
                b'x' if i + 3 < raw.len() => {
                    if let Ok(byte) = u8::from_str_radix(&ssid[i + 2..i + 4], 16) {
                        bytes.push(byte);
                        i += 4;
                        continue;
                    }
                }
                c @ (b'\\' | b'"') => {
                    bytes.push(c);
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        bytes.push(raw[i]);
        i += 1;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Where wpa_supplicant keeps the networks it knows.
const CONF: &str = "/data/wifi/wpa_supplicant.conf";

/// Add networks to wpa_supplicant's file directly, for when the radio is
/// not up yet to be asked (setup from a computer): each as a `network`
/// block, the name in hex and the passphrase quoted, as `set_network` would
/// write them. Switching Wi-Fi on afterwards reads them in.
pub fn add_networks(file: &std::path::Path, networks: &[(String, Option<String>)]) -> std::io::Result<()> {
    let mut text = std::fs::read_to_string(file).unwrap_or_else(|_| "ctrl_interface=/run/wpa_supplicant\nupdate_config=1\n".into());
    for (ssid, password) in networks {
        // What wpa_supplicant would refuse (and refusing one network, it
        // reads none): a name over 32 bytes, a passphrase not 8 to 63
        // characters, or one with a line break, which would end the block.
        let bad_name = ssid.is_empty() || ssid.len() > 32;
        let bad_password = password.as_deref().is_some_and(|p| !(8..=63).contains(&p.chars().count()) || p.contains(['\n', '\r']));
        if bad_name || bad_password {
            continue;
        }
        let secret = match password {
            Some(p) => format!("\tpsk={}\n", psk_value(p)),
            None => "\tkey_mgmt=NONE\n".to_owned(),
        };
        text.push_str(&format!("\nnetwork={{\n\tssid={}\n{secret}}}\n", ssid_value(ssid)));
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::sys::write_private(file, text.as_bytes())
}

/// The networks a computer sent during setup, kept and the radio switched on.
pub fn add_and_switch_on(networks: &[(String, Option<String>)]) {
    if networks.is_empty() {
        return;
    }
    let file = if std::path::Path::new(TOGGLE).exists() { std::path::PathBuf::from(CONF) } else { crate::config_dir().join("wpa_supplicant.conf") };
    if add_networks(&file, networks).is_ok() {
        switch(true);
    }
}

/// A network name as `set_network … ssid` takes it: as hex, unquoted, which
/// wpa_supplicant reads byte for byte. A quoted name would be taken from the
/// first quote to the last with no escapes at all, so quotes, backslashes
/// and letters beyond ASCII are safest this way.
pub fn ssid_value(ssid: &str) -> String {
    ssid.bytes().map(|b| format!("{b:02x}")).collect()
}

/// A passphrase as `set_network … psk` takes it: between double quotes,
/// exactly as typed. wpa_supplicant takes everything between the first quote
/// and the last, so quotes and backslashes inside need no escaping (and
/// escaping them would change the passphrase).
pub fn psk_value(password: &str) -> String {
    format!("\"{password}\"")
}

/// `scan_results`: bssid, frequency, signal, flags, ssid, tab-separated.
/// Access points of one network are folded into its strongest.
pub fn parse_scan(text: &str) -> Vec<Network> {
    let mut networks: Vec<Network> = Vec::new();
    for line in text.lines().skip(1) {
        let fields: Vec<&str> = line.split('\t').collect();
        let [_, _, signal, flags, ssid] = fields[..] else { continue };
        let ssid = unescape(ssid);
        if ssid.is_empty() || ssid.bytes().all(|b| b == 0) {
            continue; // hidden networks
        }
        let signal = signal.parse().unwrap_or(-100);
        let secured = ["WPA", "WEP", "SAE", "RSN"].iter().any(|k| flags.contains(k));
        match networks.iter_mut().find(|n| n.ssid == ssid) {
            Some(known) => known.signal = known.signal.max(signal),
            None => networks.push(Network { ssid, signal, secured, saved: None, current: false }),
        }
    }
    networks
}

/// `list_networks`: id, ssid, bssid, flags. Returns (id, ssid, is current).
pub fn parse_saved(text: &str) -> Vec<(u32, String, bool)> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let id = fields.first()?.parse().ok()?;
            Some((id, unescape(fields.get(1)?), fields.get(3).is_some_and(|f| f.contains("CURRENT"))))
        })
        .collect()
}

/// `status`: `key=value` lines. Returns (state, ssid, address).
pub fn parse_status(text: &str) -> (String, Option<String>, Option<String>) {
    let field = |name: &str| text.lines().find_map(|l| l.strip_prefix(name)?.strip_prefix('=')).map(str::to_owned);
    (field("wpa_state").unwrap_or_default(), field("ssid").map(|s| unescape(&s)), field("ip_address"))
}

/// The list the screen shows: saved networks marked, the current one first,
/// then by signal; saved networks out of range still listed, so they can be forgotten.
pub fn merge(mut seen: Vec<Network>, saved: &[(u32, String, bool)], current: Option<&str>) -> Vec<Network> {
    for (id, ssid, _) in saved {
        match seen.iter_mut().find(|n| &n.ssid == ssid) {
            Some(network) => network.saved = Some(*id),
            None => seen.push(Network { ssid: ssid.clone(), signal: -100, secured: true, saved: Some(*id), current: false }),
        }
    }
    for network in &mut seen {
        network.current = current == Some(network.ssid.as_str());
    }
    seen.sort_by(|a, b| b.current.cmp(&a.current).then(b.signal.cmp(&a.signal)).then(a.ssid.cmp(&b.ssid)));
    seen
}

/// What the state means, in a few words.
fn describe(state: &str, ssid: Option<&str>, address: Option<&str>) -> String {
    match (state, ssid, address) {
        ("COMPLETED", Some(ssid), Some(address)) => format!("{ssid} · {address}"),
        ("COMPLETED", Some(ssid), None) => format!("{ssid} · getting an address"),
        ("ASSOCIATING" | "ASSOCIATED" | "AUTHENTICATING" | "4WAY_HANDSHAKE" | "GROUP_HANDSHAKE", Some(ssid), _) => format!("joining {ssid}"),
        ("ASSOCIATING" | "ASSOCIATED" | "AUTHENTICATING" | "4WAY_HANDSHAKE" | "GROUP_HANDSHAKE", ..) => "joining".into(),
        ("SCANNING", ..) => "looking for networks".into(),
        ("INTERFACE_DISABLED", ..) => "wi-fi hardware is off".into(),
        // No answer from wpa_supplicant yet: the start script says why.
        ("", ..) => match service_status().as_deref() {
            Some("the radio did not start") => "the radio did not start".into(),
            _ => "starting the radio".into(),
        },
        _ => "not connected".into(),
    }
}

/// What the Wi-Fi service last said of itself (`/run/catalyst-wifi.status`):
/// starting, running, off, or why it did not start.
fn service_status() -> Option<String> {
    std::fs::read_to_string("/run/catalyst-wifi.status").ok().map(|s| s.trim().to_owned())
}

// ------------------------------------------------------------- device ---

fn cli(args: &[&str]) -> Option<String> {
    let out = Command::new(WPA_CLI).args(["-p", "/run/wpa_supplicant", "-i", "wlan0"]).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn enabled() -> bool {
    std::path::Path::new(ENABLED_FLAG).exists()
}

/// Off the handheld there is no radio: this stands in for its switch.
static DEMO_ON: AtomicBool = AtomicBool::new(false);

/// Whether Wi-Fi is switched on: all a quick switch needs to know.
pub fn switched_on() -> bool {
    if std::path::Path::new(TOGGLE).exists() {
        enabled()
    } else {
        DEMO_ON.load(Ordering::Relaxed)
    }
}

/// Switch Wi-Fi on or off without waiting for it: bringing the radio up
/// takes a few seconds.
pub fn switch(on: bool) {
    if std::path::Path::new(TOGGLE).exists() {
        thread::spawn(move || Command::new(TOGGLE).arg(if on { "on" } else { "off" }).status().ok());
    } else {
        DEMO_ON.store(on, Ordering::Relaxed);
    }
}

fn read_state() -> State {
    if !enabled() {
        return State { enabled: false, networks: Vec::new(), status: "off".into() };
    }
    let (state, ssid, address) = cli(&["status"]).map(|t| parse_status(&t)).unwrap_or_default();
    let saved = cli(&["list_networks"]).map(|t| parse_saved(&t)).unwrap_or_default();
    let seen = cli(&["scan_results"]).map(|t| parse_scan(&t)).unwrap_or_default();
    let connected = (state == "COMPLETED").then_some(ssid.as_deref()).flatten();
    State { enabled: true, networks: merge(seen, &saved, connected), status: describe(&state, ssid.as_deref(), address.as_deref()) }
}

fn join(ssid: &str, password: Option<&str>) {
    let saved = cli(&["list_networks"]).map(|t| parse_saved(&t)).unwrap_or_default();
    let id = match (saved.iter().find(|(_, name, _)| name == ssid), password) {
        // Already saved and no new password: use it as it is.
        (Some((id, ..)), None) => id.to_string(),
        (existing, _) => {
            if let Some((id, ..)) = existing {
                cli(&["remove_network", &id.to_string()]);
            }
            let Some(id) = cli(&["add_network"]).map(|t| t.trim().to_owned()).filter(|t| t.parse::<u32>().is_ok()) else { return };
            cli(&["set_network", &id, "ssid", &ssid_value(ssid)]);
            match password {
                Some(password) => cli(&["set_network", &id, "psk", &psk_value(password)]),
                None => cli(&["set_network", &id, "key_mgmt", "NONE"]),
            };
            id
        }
    };
    cli(&["select_network", &id]);
    // select_network turns the others off; keep them available for next time.
    cli(&["enable_network", "all"]);
    cli(&["save_config"]);
}

fn run(shared: &Shared, rx: &Receiver<Cmd>) {
    let mut state = read_state();
    publish(shared, state.clone());
    if enabled() {
        cli(&["scan"]);
    }
    let mut since_scan = 0u32;
    while !shared.stop.load(Ordering::Relaxed) {
        // Until the first networks are in (the radio still starting, its
        // first look round), ask every tick; after that, every 12 s.
        let every = if state.networks.is_empty() { 1 } else { 8 };
        // Quicker ticks while the list is still empty.
        let tick = if state.networks.is_empty() { 700 } else { 1500 };
        // In deep standby, parked here: no wakeups until it wakes.
        crate::deep::rest();
        match rx.recv_timeout(Duration::from_millis(tick)) {
            Ok(Cmd::Enable(on)) => {
                Command::new(TOGGLE).arg(if on { "on" } else { "off" }).status().ok();
                if on {
                    cli(&["scan"]);
                }
            }
            Ok(Cmd::Join { ssid, password }) => join(&ssid, password.as_deref()),
            Ok(Cmd::Forget(id)) => {
                cli(&["remove_network", &id.to_string()]);
                cli(&["save_config"]);
            }
            Ok(Cmd::Scan) => {
                cli(&["scan"]);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                since_scan += 1;
                // A fresh scan every 12 s while the screen is open (at once
                // while there is nothing listed yet).
                if since_scan >= every && enabled() {
                    since_scan = 0;
                    cli(&["scan"]);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        state = read_state();
        publish(shared, state.clone());
    }
}

// --------------------------------------------------------------- demo ---

fn run_demo(shared: &Shared, rx: &Receiver<Cmd>) {
    let mut enabled = true;
    let mut saved: Vec<(u32, String, bool)> = vec![(0, "5805_Robot".into(), false), (1, "Workshop".into(), false)];
    let mut current: Option<String> = Some("5805_Robot".into());
    let mut joining: Option<(String, u8)> = None;
    let seen = || {
        vec![
            Network { ssid: "5805_Robot".into(), signal: -48, secured: true, saved: None, current: false },
            Network { ssid: "Workshop".into(), signal: -61, secured: true, saved: None, current: false },
            Network { ssid: "FRC-Event-Pits".into(), signal: -70, secured: true, saved: None, current: false },
            Network { ssid: "Guest".into(), signal: -78, secured: false, saved: None, current: false },
            Network { ssid: "9999_Robot".into(), signal: -86, secured: true, saved: None, current: false },
        ]
    };
    loop {
        let status = match (&joining, &current, enabled) {
            (_, _, false) => "off".to_owned(),
            (Some((ssid, _)), ..) => format!("joining {ssid}"),
            (None, Some(ssid), _) => format!("{ssid} · 10.58.5.23"),
            (None, None, _) => "not connected".into(),
        };
        let networks = if enabled { merge(seen(), &saved, current.as_deref()) } else { Vec::new() };
        publish(shared, State { enabled, networks, status });
        if shared.stop.load(Ordering::Relaxed) {
            return;
        }
        match rx.recv_timeout(Duration::from_millis(400)) {
            Ok(Cmd::Enable(on)) => enabled = on,
            Ok(Cmd::Join { ssid, .. }) => {
                current = None;
                joining = Some((ssid, 4));
            }
            Ok(Cmd::Forget(id)) => {
                if let Some(at) = saved.iter().position(|(i, ..)| *i == id) {
                    let (_, ssid, _) = saved.remove(at);
                    if current.as_deref() == Some(ssid.as_str()) {
                        current = None;
                    }
                }
            }
            Ok(Cmd::Scan) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        // A join takes a moment, as a real one does.
        if let Some((ssid, left)) = joining.take() {
            if left == 0 {
                if !saved.iter().any(|(_, name, _)| *name == ssid) {
                    saved.push((saved.iter().map(|(id, ..)| id + 1).max().unwrap_or(0), ssid.clone(), false));
                }
                current = Some(ssid);
            } else {
                joining = Some((ssid, left - 1));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_passwords_reach_wpa_supplicant_unchanged() {
        assert_eq!(ssid_value("Pit \"5805\""), "50697420223538303522");
        assert_eq!(ssid_value("café"), "636166c3a9");
        // A passphrase goes between quotes as it is: no escapes added.
        assert_eq!(psk_value(r#"pa"ss\word"#), r#""pa"ss\word""#);
    }

    #[test]
    fn networks_from_setup_are_written_as_wpa_supplicant_reads_them() {
        let file = std::env::temp_dir().join(format!("catalyst-wpa-{}.conf", std::process::id()));
        std::fs::remove_file(&file).ok();
        let networks = [
            ("Pit".into(), Some("hunter22".into())),
            ("Open".into(), None),
            ("Bad".into(), Some("hunter22\nx".into())),
            ("Short".into(), Some("abc".into())),
        ];
        add_networks(&file, &networks).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("ctrl_interface=/run/wpa_supplicant\nupdate_config=1\n"));
        assert!(text.contains("network={\n\tssid=506974\n\tpsk=\"hunter22\"\n}"));
        assert!(text.contains("network={\n\tssid=4f70656e\n\tkey_mgmt=NONE\n}"));
        assert!(!text.contains("426164"), "a passphrase with a line break is not written");
        assert!(!text.contains("53686f7274"), "nor one wpa_supplicant would refuse as too short");
        std::fs::remove_file(&file).ok();
    }

    const SCAN: &str = "bssid / frequency / signal level / flags / ssid\n\
aa:bb:cc:00:00:01\t2437\t-52\t[WPA2-PSK-CCMP][ESS]\t5805_Robot\n\
aa:bb:cc:00:00:02\t5180\t-47\t[WPA2-PSK-CCMP][ESS]\t5805_Robot\n\
aa:bb:cc:00:00:03\t2412\t-71\t[ESS]\tGuest\n\
aa:bb:cc:00:00:04\t2462\t-80\t[WPA2-SAE-CCMP][ESS]\tCaf\\xc3\\xa9 \\\"Net\\\"\n\
aa:bb:cc:00:00:05\t2462\t-60\t[WPA2-PSK-CCMP][ESS]\t\n";

    #[test]
    fn a_scan_becomes_networks() {
        let networks = parse_scan(SCAN);
        assert_eq!(networks.len(), 3, "the hidden network is left out and the two access points fold into one");
        assert_eq!((networks[0].ssid.as_str(), networks[0].signal, networks[0].secured), ("5805_Robot", -47, true));
        assert_eq!((networks[1].ssid.as_str(), networks[1].secured), ("Guest", false));
        assert_eq!(networks[2].ssid, "Café \"Net\"");
        assert_eq!((networks[0].bars(), networks[1].bars(), networks[2].bars()), (4, 2, 1));
    }

    #[test]
    fn online_means_joined_with_an_address() {
        let state = |status: &str| State { enabled: true, networks: Vec::new(), status: status.into() };
        assert!(state("5805_Robot · 10.58.5.23").online());
        assert!(state(&describe("COMPLETED", Some("Pits · Guest"), Some("192.168.4.20"))).online(), "a name with a dot in it");
        assert!(!state(&describe("COMPLETED", Some("Pits"), None)).online(), "still getting an address");
        assert!(!state(&describe("ASSOCIATING", Some("Pits"), None)).online());
        assert!(!state("starting the radio").online());
        assert!(!State { enabled: false, ..state("Pits · 10.0.0.2") }.online());
    }

    #[test]
    fn saved_networks_and_status_are_read() {
        let saved = parse_saved("network id / ssid / bssid / flags\n0\t5805_Robot\tany\t[CURRENT]\n1\tWorkshop\tany\t[DISABLED]\n");
        assert_eq!(saved, vec![(0, "5805_Robot".to_owned(), true), (1, "Workshop".to_owned(), false)]);
        let (state, ssid, address) = parse_status("bssid=aa:bb\nfreq=2437\nssid=5805_Robot\nid=0\nwpa_state=COMPLETED\nip_address=10.58.5.23\n");
        assert_eq!((state.as_str(), ssid.as_deref(), address.as_deref()), ("COMPLETED", Some("5805_Robot"), Some("10.58.5.23")));
        assert_eq!(describe(&state, ssid.as_deref(), address.as_deref()), "5805_Robot · 10.58.5.23");
        assert_eq!(describe("4WAY_HANDSHAKE", Some("Workshop"), None), "joining Workshop");
        assert_eq!(describe("DISCONNECTED", None, None), "not connected");
    }

    #[test]
    fn the_list_puts_the_current_network_first_and_keeps_saved_ones() {
        let saved = vec![(0, "Guest".to_owned(), false), (7, "Home".to_owned(), false)];
        let list = merge(parse_scan(SCAN), &saved, Some("Guest"));
        let names: Vec<&str> = list.iter().map(|n| n.ssid.as_str()).collect();
        assert_eq!(names, ["Guest", "5805_Robot", "Café \"Net\"", "Home"]);
        assert!(list[0].current && list[0].saved == Some(0));
        // Saved but out of range: still listed, so it can be forgotten.
        assert_eq!((list[3].saved, list[3].signal), (Some(7), -100));
    }
}
