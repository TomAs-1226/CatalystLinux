//! Connect: other machines, from the handheld. Saved SSH hosts, each opened
//! in a terminal through Dropbear's client (`dbclient`), with a key of the
//! handheld's own to give servers; and serial ports (USB adapters, a
//! microcontroller, a robot's console) at the speed chosen, raw.
//!
//! Passwords and a new server's fingerprint are asked for in the terminal,
//! as `ssh` asks for them anywhere. Ctrl-Q (or Esc held) leaves a session.

use std::path::{Path, PathBuf};

use bezel::{Align, Rect};

use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{heading, host_char, shell_quote, step_over_headings, Field, Typed};

/// The speeds a serial port is offered at.
pub const BAUDS: [u32; 8] = [9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600];

// ----------------------------------------------------------------- hosts ---

/// A machine to log into: who as, where, on which port.
#[derive(Clone, Debug, PartialEq)]
pub struct Host {
    pub user: String,
    pub host: String,
    pub port: u16,
}

impl Host {
    /// `user@host`, `user@host:port` or `host`.
    pub fn parse(text: &str) -> Option<Host> {
        let text = text.trim();
        let (user, rest) = match text.split_once('@') {
            Some((u, r)) if !u.is_empty() => (u.to_owned(), r),
            Some(_) => return None,
            None => (String::new(), text),
        };
        let (host, port) = match rest.rsplit_once(':') {
            Some((h, p)) if !h.contains(':') => (h, p.parse::<u16>().ok().filter(|p| *p > 0)?),
            _ => (rest, 22),
        };
        let fine = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c));
        (fine(host) && (user.is_empty() || fine(&user))).then(|| Host { user, host: host.to_owned(), port })
    }

    /// As it is written down: `user@host:port`.
    pub fn line(&self) -> String {
        let who = if self.user.is_empty() { self.host.clone() } else { format!("{}@{}", self.user, self.host) };
        if self.port == 22 { who } else { format!("{who}:{}", self.port) }
    }
}

fn hosts_file() -> PathBuf {
    crate::config_dir().join("ssh-hosts")
}

pub fn load_hosts() -> Vec<Host> {
    std::fs::read_to_string(hosts_file()).unwrap_or_default().lines().filter_map(Host::parse).collect()
}

fn save_hosts(hosts: &[Host]) -> bool {
    let text: String = hosts.iter().map(|h| h.line() + "\n").collect();
    std::fs::create_dir_all(crate::config_dir()).ok();
    crate::sys::write_atomic(&hosts_file(), text.as_bytes()).is_ok()
}

// ------------------------------------------------------------------- key ---

/// Where SSH things are kept: beside the keys allowed in (`/data/ssh` on
/// the handheld), else beside the settings.
fn ssh_dir() -> PathBuf {
    if cfg!(target_os = "linux") && Path::new("/data").is_dir() {
        PathBuf::from("/data/ssh")
    } else {
        crate::config_dir().join("ssh")
    }
}

/// The handheld's own key, which servers are given the public half of.
fn key_path() -> PathBuf {
    ssh_dir().join("id_dropbear")
}

/// The public half, as a server's `authorized_keys` takes it, if there is a key.
pub fn public_key() -> Option<String> {
    let text = std::fs::read_to_string(key_path().with_extension("pub")).ok()?;
    public_line(&text)
}

/// The `ssh-…` line in what `dropbearkey -y` prints.
pub fn public_line(text: &str) -> Option<String> {
    text.lines().map(str::trim).find(|l| l.starts_with("ssh-") || l.starts_with("ecdsa-")).map(str::to_owned)
}

/// Make the handheld's key, once, and write its public half beside it.
fn make_key() -> Result<String, String> {
    if let Some(public) = public_key() {
        return Ok(public);
    }
    let path = key_path();
    std::fs::create_dir_all(ssh_dir()).map_err(|_| "cannot make the ssh folder".to_owned())?;
    let made = crate::tools::quiet("dropbearkey").args(["-t", "ed25519", "-f"]).arg(&path).output();
    match made {
        Err(_) => return Err("keys are made on the handheld".into()),
        Ok(out) if !out.status.success() && !path.exists() => return Err("dropbearkey could not make a key".into()),
        Ok(_) => {}
    }
    let out = crate::tools::quiet("dropbearkey").arg("-y").arg("-f").arg(&path).output().map_err(|_| "cannot read the key".to_owned())?;
    let public = public_line(&String::from_utf8_lossy(&out.stdout)).ok_or("cannot read the key")?;
    std::fs::write(path.with_extension("pub"), format!("{public}\n")).ok();
    Ok(public)
}

/// The command line that logs into `host`: Dropbear's client, with the
/// handheld's key when it has one. Its home is on the data partition, so
/// the servers it has met (`known_hosts`) are remembered; the root
/// filesystem is read-only.
pub fn ssh_command(host: &Host, key: Option<&Path>) -> String {
    let home = ssh_dir().join("client");
    let mut line = format!("HOME={} dbclient -p {}", shell_quote(&home.to_string_lossy()), host.port);
    if let Some(key) = key {
        line += &format!(" -i {}", shell_quote(&key.to_string_lossy()));
    }
    let who = if host.user.is_empty() { host.host.clone() } else { format!("{}@{}", host.user, host.host) };
    line + " " + &shell_quote(&who)
}

// ---------------------------------------------------------------- serial ---

/// A serial port: a name to show and its device.
#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    pub name: String,
    pub path: String,
}

/// A name for a port from its `/dev/serial/by-id` entry:
/// `usb-FTDI_FT232R_USB_UART_A50285BI-if00-port0` is "FTDI FT232R USB UART".
pub fn friendly(by_id: &str) -> String {
    let mut s = by_id.strip_prefix("usb-").unwrap_or(by_id);
    for _ in 0..2 {
        if let Some((front, tail)) = s.rsplit_once('-') {
            if tail.starts_with("if") || tail.starts_with("port") {
                s = front;
            }
        }
    }
    let mut words: Vec<&str> = s.split('_').filter(|w| !w.is_empty()).collect();
    // The adapter's serial number, last: long, and mostly digits.
    if words.len() > 1 {
        let last = words[words.len() - 1];
        if last.len() >= 6 && last.chars().filter(|c| c.is_ascii_digit()).count() * 2 >= last.len() {
            words.pop();
        }
    }
    words.join(" ")
}

/// The serial ports there are now: USB adapters and boards, by their own
/// names where the system knows them. On a computer, a stand-in.
pub fn ports() -> Vec<Port> {
    if !cfg!(target_os = "linux") {
        return vec![Port { name: "Stand-in device".into(), path: "/dev/ttyUSB0".into() }];
    }
    let mut found: Vec<Port> = Vec::new();
    if let Ok(dir) = std::fs::read_dir("/dev/serial/by-id") {
        for entry in dir.flatten() {
            let Ok(target) = std::fs::canonicalize(entry.path()) else { continue };
            found.push(Port { name: friendly(&entry.file_name().to_string_lossy()), path: target.to_string_lossy().into_owned() });
        }
    }
    if let Ok(dir) = std::fs::read_dir("/dev") {
        let mut plain: Vec<String> = dir
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("ttyUSB") || n.starts_with("ttyACM"))
            .map(|n| format!("/dev/{n}"))
            .filter(|p| !found.iter().any(|f| f.path == *p))
            .collect();
        plain.sort();
        found.extend(plain.into_iter().map(|path| Port { name: path.trim_start_matches("/dev/").to_owned(), path }));
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

fn baud_file() -> PathBuf {
    crate::config_dir().join("serial-speed")
}

// ---------------------------------------------------------------- screen ---

#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    Heading(&'static str),
    Host(usize),
    AddHost,
    Key,
    Port(usize),
    NoPorts,
    Speed,
}

/// What a typed field is for.
#[derive(Clone, Copy, PartialEq)]
enum Typing {
    NewHost,
    EditHost(usize),
}

pub struct Connect {
    hosts: Vec<Host>,
    ports: Vec<Port>,
    baud: u32,
    list: ListState,
    field: Option<(Field, Typing)>,
    /// A host `d` was pressed on once; pressed again, it goes.
    deleting: Option<usize>,
    looked: std::time::Instant,
}

impl Connect {
    pub fn new() -> Self {
        let baud = std::fs::read_to_string(baud_file()).ok().and_then(|s| s.trim().parse().ok()).filter(|b| BAUDS.contains(b)).unwrap_or(115200);
        let mut connect =
            Connect { hosts: load_hosts(), ports: ports(), baud, list: ListState::default(), field: None, deleting: None, looked: std::time::Instant::now() };
        connect.list.focus = 1;
        connect
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::Heading("ssh")];
        rows.extend((0..self.hosts.len()).map(Row::Host));
        rows.extend([Row::AddHost, Row::Key, Row::Heading("serial")]);
        if self.ports.is_empty() {
            rows.push(Row::NoPorts);
        } else {
            rows.extend((0..self.ports.len()).map(Row::Port));
        }
        rows.push(Row::Speed);
        rows
    }

    fn step_baud(&mut self, step: i32) {
        let at = BAUDS.iter().position(|b| *b == self.baud).unwrap_or(4) as i32;
        self.baud = BAUDS[(at + step).clamp(0, BAUDS.len() as i32 - 1) as usize];
        std::fs::create_dir_all(crate::config_dir()).ok();
        std::fs::write(baud_file(), self.baud.to_string()).ok();
    }
}

/// Where a terminal session starts: the data partition, or the home folder.
pub fn session_dir() -> PathBuf {
    if Path::new("/data").is_dir() {
        PathBuf::from("/data")
    } else {
        std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
    }
}

impl Screen for Connect {
    fn title(&self) -> &str {
        "Connect"
    }

    fn wants_text(&self) -> bool {
        self.field.is_some()
    }

    fn resume(&mut self) {
        self.ports = ports();
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some((field, typing)) = &mut self.field {
            let typing = *typing;
            match field.key(key, host_char) {
                Typed::Done(text) => match Host::parse(&text) {
                    Some(host) => {
                        match typing {
                            Typing::NewHost => self.hosts.push(host),
                            Typing::EditHost(i) => self.hosts[i] = host,
                        }
                        self.field = None;
                        *cx.say = Some(if save_hosts(&self.hosts) { "host kept".into() } else { "could not keep the host".into() });
                        let at = match typing {
                            Typing::NewHost => self.hosts.len() - 1,
                            Typing::EditHost(i) => i,
                        };
                        self.list.focus = self.rows().iter().position(|r| *r == Row::Host(at)).unwrap_or(1);
                    }
                    None => *cx.say = Some("as user@host, or user@host:port".into()),
                },
                Typed::Cancelled => self.field = None,
                Typed::More => {}
            }
            return Nav::Stay;
        }
        let rows = self.rows();
        let row = rows.get(self.list.focus).copied().unwrap_or(Row::AddHost);
        let armed = self.deleting.take();
        match (key, row) {
            (Key::Back, _) => return Nav::Pop,
            (Key::Enter | Key::Right, Row::Host(i)) => {
                let host = &self.hosts[i];
                let key = key_path().exists().then(key_path);
                std::fs::create_dir_all(ssh_dir().join("client").join(".ssh")).ok();
                let line = ssh_command(host, key.as_deref());
                let session = crate::terminal::Terminal::running("SSH", &line, session_dir()).with_banner(&format!("ssh {}", host.line()));
                return Nav::Push(Box::new(session));
            }
            (Key::Enter | Key::Right, Row::AddHost) => self.field = Some((Field::new("user@host, or user@host:port", "", 64), Typing::NewHost)),
            (Key::Enter | Key::Right, Row::Key) => match make_key() {
                Ok(public) => return Nav::Push(Box::new(KeyScreen { public })),
                Err(why) => *cx.say = Some(why),
            },
            (Key::Enter | Key::Right, Row::Port(i)) => {
                let port = &self.ports[i];
                let banner = format!("{} · {} · {} baud", port.name, port.path, self.baud);
                return Nav::Push(Box::new(crate::terminal::Terminal::serial("Serial", &port.path, self.baud).with_banner(&banner)));
            }
            (Key::Enter | Key::Right, Row::Speed) => self.step_baud(1),
            (Key::Left, Row::Speed) => self.step_baud(-1),
            (Key::Char('e'), Row::Host(i)) => self.field = Some((Field::new("user@host, or user@host:port", &self.hosts[i].line(), 64), Typing::EditHost(i))),
            (Key::Char('d'), Row::Host(i)) => {
                if armed == Some(i) {
                    let gone = self.hosts.remove(i);
                    save_hosts(&self.hosts);
                    *cx.say = Some(format!("forgot {}", gone.host));
                    // The focus stays where the host was: on the next one, or on Add a host.
                    self.list.focus = self.list.focus.min(self.rows().len() - 1);
                } else {
                    *cx.say = Some(format!("d again forgets {}", self.hosts[i].host));
                    self.deleting = Some(i);
                }
            }
            (other, _) => {
                let rows = self.rows();
                step_over_headings(&mut self.list, other, rows.len(), |i| matches!(rows[i], Row::Heading(_)));
            }
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        // An adapter plugged in or pulled out shows within a couple of seconds.
        if self.looked.elapsed().as_secs_f32() < 2.0 {
            return false;
        }
        self.looked = std::time::Instant::now();
        let now = ports();
        if now != self.ports {
            self.ports = now;
            self.list.focus = self.list.focus.min(self.rows().len() - 1);
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        if let Some((field, _)) = &self.field {
            return field.draw(g, "enter: keep it · esc: cancel");
        }
        let rows = self.rows();
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        let has_key = key_path().with_extension("pub").exists();
        for (i, y) in self.list.layout(g, rows.len(), top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let focused = i == self.list.focus;
            let (name, value, ink) = match rows[i] {
                Row::Heading(name) => {
                    heading(g, y, row_h, name);
                    continue;
                }
                Row::Host(h) => {
                    let host = &self.hosts[h];
                    let who = if host.user.is_empty() { host.host.clone() } else { format!("{}@{}", host.user, host.host) };
                    (who, if host.port == 22 { "›".into() } else { format!(":{} ›", host.port) }, t.dim)
                }
                Row::AddHost => ("Add a host".into(), "user@host".into(), t.dim),
                Row::Key => ("Key for servers".into(), (if has_key { "ed25519 ›" } else { "make one" }).into(), if has_key { t.ice } else { t.dim }),
                Row::Port(p) => {
                    let port = &self.ports[p];
                    (port.name.clone(), port.path.trim_start_matches("/dev/").to_owned(), t.dim)
                }
                Row::NoPorts => ("No serial adapters".into(), "plug one in".into(), t.faint),
                Row::Speed => ("Speed".into(), format!("{} baud", self.baud), t.ink),
            };
            g.row(r, &name, &value, ink, focused);
        }
        self.list.scrollbar(g, rows.len(), top, bottom);
        let hint = match rows.get(self.list.focus) {
            Some(Row::Host(_)) => "enter: connect · e: edit · d: forget",
            Some(Row::Port(_)) => "enter: open · ctrl q: leave it",
            Some(Row::Speed) => "←→: speed",
            Some(Row::Key) if has_key => "enter: show it",
            Some(Row::Key) => "enter: make the handheld's key",
            _ => "",
        };
        g.hint(hint);
    }
}

/// The public half of the handheld's key, to put in a server's
/// `~/.ssh/authorized_keys`.
pub struct KeyScreen {
    public: String,
}

impl Screen for KeyScreen {
    fn title(&self) -> &str {
        "Key"
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        if matches!(key, Key::Back | Key::Left | Key::Enter) { Nav::Pop } else { Nav::Stay }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let mut parts = self.public.split_whitespace();
        let (kind, body, comment) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        let panel = Rect::new(PAD, top, W - 2.0 * PAD, H - PAD - g.hint_h() - top);
        g.surface(panel, g.tile_radius(), t.surface1);
        let style = ty.caption;
        g.text_at(panel.x + 9.0, panel.y + 6.0 + style.size * 0.5, Align::Left, &format!("{kind} · {comment}"), style, t.dim);
        // The key itself, broken where it meets the edge, as it is: every character counts.
        let cell = g.measure("0", style);
        let per_line = ((panel.w - 18.0) / cell).floor().max(8.0) as usize;
        let chars: Vec<char> = body.chars().collect();
        let line_h = style.size + 3.0;
        for (n, piece) in chars.chunks(per_line).enumerate() {
            let line: String = piece.iter().collect();
            g.text_at(panel.x + 9.0, panel.y + 10.0 + style.size + (n as f32 + 0.5) * line_h, Align::Left, &line, style, t.ink);
        }
        let file = key_path().with_extension("pub");
        g.hint(&format!("also in {}", file.display()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_is_written_as_ssh_writes_it() {
        assert_eq!(Host::parse("admin@10.58.5.2"), Some(Host { user: "admin".into(), host: "10.58.5.2".into(), port: 22 }));
        assert_eq!(Host::parse("pi@build.local:2222").unwrap().port, 2222);
        assert_eq!(Host::parse("server").unwrap().user, "");
        assert_eq!(Host::parse("@nobody"), None);
        assert_eq!(Host::parse("a@b:0"), None);
        assert_eq!(Host::parse("a@b:x"), None);
        assert_eq!(Host::parse("a b@c"), None);
        for line in ["admin@10.58.5.2", "pi@build.local:2222", "server"] {
            assert_eq!(Host::parse(line).unwrap().line(), line, "written back as it was");
        }
    }

    #[test]
    fn the_ssh_command_is_dropbears_with_the_handhelds_key() {
        let host = Host::parse("admin@10.58.5.2:2222").unwrap();
        let line = ssh_command(&host, Some(Path::new("/data/ssh/id_dropbear")));
        assert!(line.contains("dbclient -p 2222 -i '/data/ssh/id_dropbear' 'admin@10.58.5.2'"), "{line}");
        assert!(line.starts_with("HOME='"), "a home it can write known hosts to");
        assert!(!ssh_command(&host, None).contains(" -i "));
    }

    #[test]
    fn a_port_is_named_after_its_adapter() {
        assert_eq!(friendly("usb-FTDI_FT232R_USB_UART_A50285BI-if00-port0"), "FTDI FT232R USB UART");
        assert_eq!(friendly("usb-1a86_USB_Serial-if00-port0"), "1a86 USB Serial");
        assert_eq!(friendly("usb-Raspberry_Pi_Pico_E6614C311B5F4A2B-if00"), "Raspberry Pi Pico");
    }

    #[test]
    fn the_public_half_is_found_in_what_dropbearkey_says() {
        let said = "Public key portion is:\nssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI root@catalyst\nFingerprint: sha256!abc\n";
        assert_eq!(public_line(said).as_deref(), Some("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI root@catalyst"));
        assert_eq!(public_line("nothing here"), None);
    }

    #[test]
    fn hosts_are_kept_and_the_speed_steps_through_what_ports_take() {
        crate::shell::tests::sandbox();
        let mut connect = Connect::new();
        connect.hosts = vec![Host::parse("admin@10.58.5.2").unwrap()];
        assert!(save_hosts(&connect.hosts));
        assert_eq!(load_hosts(), connect.hosts);
        connect.baud = 115200;
        connect.step_baud(1);
        assert_eq!(connect.baud, 230400);
        for _ in 0..20 {
            connect.step_baud(1);
        }
        assert_eq!(connect.baud, 921600, "no faster than the last");
        // The headings are never focused.
        let rows = connect.rows();
        for _ in 0..rows.len() * 2 {
            step_over_headings(&mut connect.list, Key::Down, rows.len(), |i| matches!(rows[i], Row::Heading(_)));
            assert!(!matches!(rows[connect.list.focus], Row::Heading(_)));
        }
        save_hosts(&[]);
    }
}
