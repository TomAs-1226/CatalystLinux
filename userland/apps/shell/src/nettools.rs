//! Net tools: the robot finder, ping, a port check, a name lookup and a
//! Wi-Fi survey. Each works on a thread of its own that lives only while
//! its page is open, and the page shows what it has found so far.
//!
//! Everything here works in the simulator too, against the computer's own
//! network (the Wi-Fi survey shows the scripted networks there).

use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bezel::Rect;

use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{host_char, Field, Typed};

// ------------------------------------------------------------------ jobs ---

/// Work on a thread of its own, with what it has found so far shared with
/// the page. It is told to stop when the page goes.
struct Job<T> {
    shared: Arc<Shared<T>>,
}

struct Shared<T> {
    found: Mutex<T>,
    generation: AtomicU64,
    stop: AtomicBool,
}

/// The thread's side of a job.
struct Report<T> {
    shared: Arc<Shared<T>>,
}

impl<T> Report<T> {
    fn update(&self, f: impl FnOnce(&mut T)) {
        f(&mut self.shared.found.lock().unwrap());
        self.shared.generation.fetch_add(1, Ordering::Relaxed);
    }

    fn stopped(&self) -> bool {
        self.shared.stop.load(Ordering::Relaxed)
    }

    /// Wait up to `d`, a little at a time, so a stop is not kept waiting.
    fn wait(&self, d: Duration) {
        let until = Instant::now() + d;
        while !self.stopped() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50).min(until.saturating_duration_since(Instant::now())));
        }
    }
}

impl<T> Clone for Report<T> {
    fn clone(&self) -> Self {
        Report { shared: self.shared.clone() }
    }
}

impl<T: Clone + Send + 'static> Job<T> {
    fn start(start: T, work: impl FnOnce(Report<T>) + Send + 'static) -> Job<T> {
        let shared = Arc::new(Shared { found: Mutex::new(start), generation: AtomicU64::new(0), stop: AtomicBool::new(false) });
        let report = Report { shared: shared.clone() };
        std::thread::Builder::new().name("net-tools".into()).stack_size(128 * 1024).spawn(move || work(report)).ok();
        Job { shared }
    }

    fn get(&self) -> T {
        self.shared.found.lock().unwrap().clone()
    }

    fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Relaxed)
    }
}

impl<T> Drop for Job<T> {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

/// Spread `work` over up to `threads` threads, one item at a time each.
fn each<I: Send + 'static>(items: Vec<I>, threads: usize, work: impl Fn(I) + Send + Sync + 'static) {
    let queue = Arc::new(Mutex::new(items));
    let work = Arc::new(work);
    let handles: Vec<_> = (0..threads)
        .filter_map(|_| {
            let (queue, work) = (queue.clone(), work.clone());
            std::thread::Builder::new()
                .stack_size(96 * 1024)
                .spawn(move || loop {
                    let Some(item) = queue.lock().unwrap().pop() else { return };
                    work(item);
                })
                .ok()
        })
        .collect();
    for h in handles {
        h.join().ok();
    }
}

// --------------------------------------------------------------- probing ---

/// How long a port is given to answer.
const PORT_WAIT: Duration = Duration::from_millis(450);

/// What a port said.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Port {
    /// Not asked yet.
    Asking,
    /// It took the connection, in this many milliseconds.
    Open(f32),
    /// The machine is there and refused it.
    Closed,
    /// Nothing came back in time.
    Silent,
}

/// Knock on a port.
pub fn knock(addr: SocketAddr) -> Port {
    let started = Instant::now();
    match TcpStream::connect_timeout(&addr, PORT_WAIT) {
        Ok(_) => Port::Open(started.elapsed().as_secs_f32() * 1000.0),
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => Port::Closed,
        Err(_) => Port::Silent,
    }
}

/// A name or an address to the addresses it stands for.
pub fn resolve(host: &str) -> Result<Vec<IpAddr>, String> {
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![ip]);
    }
    let mut found: Vec<IpAddr> = (host, 0).to_socket_addrs().map_err(|_| format!("no address for {host}"))?.map(|a| a.ip()).collect();
    found.dedup();
    if found.is_empty() {
        return Err(format!("no address for {host}"));
    }
    Ok(found)
}

/// The time a ping reply took, from what `ping` printed: BusyBox's
/// `time=0.123 ms`, Windows' `time=1ms` or `time<1ms`. `None` without a reply.
pub fn parse_ping(text: &str) -> Option<f32> {
    let lower = text.to_ascii_lowercase();
    // A reply carries its time to live; "unreachable" does not.
    if !lower.contains("ttl=") {
        return None;
    }
    let at = lower.find("time")? + 4;
    let rest = &lower[at..];
    let under = rest.starts_with('<');
    let digits: String = rest.trim_start_matches(['=', '<']).chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let ms: f32 = digits.parse().ok()?;
    Some(if under { ms * 0.5 } else { ms })
}

/// One ping, through the system's own `ping`, waiting up to a second.
/// `Err` when there is no `ping` to be had.
pub fn ping_once(host: &str) -> Result<Option<f32>, String> {
    let mut command = crate::tools::quiet("ping");
    if cfg!(windows) {
        command.args(["-n", "1", "-w", "1000", host]);
    } else {
        command.args(["-c", "1", "-W", "1", host]);
    }
    let out = command.output().map_err(|_| "no ping on this system".to_owned())?;
    Ok(parse_ping(&String::from_utf8_lossy(&out.stdout)))
}

/// Ports written as a list: `22 80 5800-5810`, commas or spaces between.
pub fn parse_ports(spec: &str) -> Result<Vec<u16>, String> {
    let mut ports = Vec::new();
    for part in spec.split([',', ' ']).filter(|p| !p.is_empty()) {
        let bad = || format!("{part} is not a port");
        match part.split_once('-') {
            Some((a, b)) => {
                let (a, b): (u16, u16) = (a.parse().map_err(|_| bad())?, b.parse().map_err(|_| bad())?);
                if a == 0 || b < a || b - a > 256 {
                    return Err(bad());
                }
                ports.extend(a..=b);
            }
            None => ports.push(part.parse().ok().filter(|p| *p > 0).ok_or_else(bad)?),
        }
    }
    ports.dedup();
    if ports.is_empty() {
        return Err("no ports".into());
    }
    Ok(ports)
}

/// What a port is usually for, where it is one a robot or a computer uses.
pub fn port_name(port: u16) -> Option<&'static str> {
    Some(match port {
        22 => "ssh",
        53 => "dns",
        80 => "web",
        443 => "https",
        1180..=1190 => "camera",
        1735 => "nt3",
        5800..=5809 => "team",
        5810 => "nt4",
        crate::agent::PORT => "agent",
        _ => return None,
    })
}

/// The ports a robot is checked on, by default.
pub const ROBOT_PORTS: &str = "22 80 443 1181 1735 5800 5810 9010";

// ------------------------------------------------------------------ menu ---

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Finder,
    Ping,
    Ports,
    Lookup,
    Survey,
}

const TOOLS: [(Tool, &str, &str); 5] = [
    (Tool::Finder, "Find the robot", "sweep"),
    (Tool::Ping, "Ping", "a host"),
    (Tool::Ports, "Ports", "open or not"),
    (Tool::Lookup, "Name lookup", "dns"),
    (Tool::Survey, "Wi-Fi survey", "signal"),
];

/// The tools, as a list.
#[derive(Default)]
pub struct NetTools {
    list: ListState,
}

impl NetTools {
    pub fn new() -> Self {
        NetTools::default()
    }
}

impl Screen for NetTools {
    fn title(&self) -> &str {
        "Net tools"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            Key::Back => Nav::Pop,
            Key::Enter | Key::Right => {
                let robot = crate::tools::robot_address(cx.robot);
                let team = crate::tools::team(cx.robot);
                Nav::Push(match TOOLS[self.list.focus].0 {
                    Tool::Finder => Box::new(Finder::new(team)),
                    Tool::Ping => Box::new(Ping::new(robot)),
                    Tool::Ports => Box::new(Ports::new(robot)),
                    Tool::Lookup => Box::new(Lookup::new(team)),
                    Tool::Survey => Box::new(Survey::new()),
                })
            }
            other => {
                crate::screens::list_key(&mut self.list, other, TOOLS.len());
                Nav::Stay
            }
        }
    }

    fn draw(&mut self, g: &mut Gfx, robot: &Robot) {
        let t = g.tokens;
        let (top, row_h) = (g.top(), g.row_h());
        let team = crate::tools::team(robot);
        for (i, y) in self.list.layout(g, TOOLS.len(), top, row_h, 3.0, H) {
            let (tool, name, about) = TOOLS[i];
            let value = match (tool, team) {
                (Tool::Finder, Some(team)) => format!("team {team} ›"),
                _ => format!("{about} ›"),
            };
            g.row(Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h), name, &value, t.dim, i == self.list.focus);
        }
        self.list.scrollbar(g, TOOLS.len(), top, H);
    }
}

// ---------------------------------------------------------------- finder ---

/// Where a team's robot may be found, and what each place is.
pub fn places(team: u32) -> Vec<(&'static str, String)> {
    let (te, am) = (team / 100, team % 100);
    vec![
        ("Robot", format!("10.{te}.{am}.2")),
        ("roboRIO by name", format!("roborio-{team}-frc.local")),
        ("roboRIO over USB", "172.22.11.2".into()),
        ("Radio", format!("10.{te}.{am}.1")),
    ]
}

/// What the robot is checked for, in the order it is said.
const SERVICES: [(u16, &str); 5] = [(5810, "nt4"), (crate::agent::PORT, "agent"), (22, "ssh"), (80, "web"), (1735, "nt3")];

#[derive(Clone, Debug, PartialEq)]
struct Place {
    label: &'static str,
    host: String,
    /// Done looking; what answered, and how long a ping took.
    done: bool,
    services: Vec<&'static str>,
    ping: Option<f32>,
}

impl Place {
    fn answered(&self) -> bool {
        !self.services.is_empty() || self.ping.is_some()
    }

    /// A robot the shell can link to: NetworkTables or the agent answered.
    fn robot(&self) -> bool {
        self.services.iter().any(|s| *s == "nt4" || *s == "nt3" || *s == "agent")
    }
}

/// Every place a team's robot may be, looked at all at once.
pub struct Finder {
    team: Option<u32>,
    field: Option<Field>,
    job: Option<Job<Vec<Place>>>,
    seen: u64,
    list: ListState,
}

impl Finder {
    fn new(team: Option<u32>) -> Self {
        let mut finder = Finder { team, field: None, job: None, seen: u64::MAX, list: ListState::default() };
        match team {
            Some(team) => finder.search(team),
            None => finder.field = Some(Field::new("team number", "", 5)),
        }
        finder
    }

    fn search(&mut self, team: u32) {
        self.team = Some(team);
        let start: Vec<Place> =
            places(team).into_iter().map(|(label, host)| Place { label, host, done: false, services: Vec::new(), ping: None }).collect();
        let hosts: Vec<(usize, String)> = start.iter().enumerate().map(|(i, p)| (i, p.host.clone())).collect();
        self.job = Some(Job::start(start, move |report| {
            each(hosts, 4, move |(i, host)| {
                let addresses = resolve(&host).unwrap_or_default();
                let ping = ping_once(&host).ok().flatten();
                report.update(|places| places[i].ping = ping);
                let mut services = Vec::new();
                if let Some(ip) = addresses.first() {
                    for (port, name) in SERVICES {
                        if report.stopped() {
                            return;
                        }
                        if matches!(knock(SocketAddr::new(*ip, port)), Port::Open(_)) {
                            services.push(name);
                            report.update(|places| places[i].services = services.clone());
                        }
                    }
                }
                report.update(|places| places[i].done = true);
            });
        }));
    }
}

impl Screen for Finder {
    fn title(&self) -> &str {
        "Finder"
    }

    fn wants_text(&self) -> bool {
        self.field.is_some()
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some(field) = &mut self.field {
            match field.key(key, |c| c.is_ascii_digit()) {
                Typed::Done(text) => match text.parse::<u32>().ok().filter(|t| *t > 0 && *t < 100_000) {
                    Some(team) => {
                        self.field = None;
                        self.search(team);
                    }
                    None => *cx.say = Some("a team number, up to five digits".into()),
                },
                Typed::Cancelled if self.team.is_none() => return Nav::Pop,
                Typed::Cancelled => self.field = None,
                Typed::More => {}
            }
            return Nav::Stay;
        }
        let places = self.job.as_ref().map(Job::get).unwrap_or_default();
        match key {
            Key::Back => return Nav::Pop,
            Key::Char('r') => {
                if let Some(team) = self.team {
                    self.search(team);
                    *cx.say = Some("looking again".into());
                }
            }
            Key::Char('t') => self.field = Some(Field::new("team number", &self.team.map_or(String::new(), |t| t.to_string()), 5)),
            Key::Enter => match places.get(self.list.focus) {
                Some(place) if place.robot() => {
                    *cx.robot = Robot::connect(&place.host);
                    crate::save_robot_address(&place.host);
                    *cx.say = Some(format!("robot set to {}", place.host));
                    return Nav::Pop;
                }
                Some(place) if place.done => *cx.say = Some(format!("no robot at {}", place.host)),
                Some(_) => *cx.say = Some("still looking".into()),
                None => {}
            },
            other => crate::screens::list_key(&mut self.list, other, places.len()),
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        let generation = self.job.as_ref().map_or(0, Job::generation);
        generation != std::mem::replace(&mut self.seen, generation)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        if let Some(field) = &self.field {
            return field.draw(g, "enter: look for it · esc: back");
        }
        let places = self.job.as_ref().map(Job::get).unwrap_or_default();
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        for (i, y) in self.list.layout(g, places.len(), top, row_h, 3.0, bottom) {
            let p = &places[i];
            let value = match (p.done, p.services.is_empty(), p.ping) {
                (_, false, _) => p.services.join(" · "),
                (true, true, Some(ms)) => format!("ping {ms:.0} ms"),
                (true, true, None) => "no answer".into(),
                (false, true, _) => "looking…".into(),
            };
            let ink = if p.robot() { t.ice } else if p.answered() { t.ink } else { t.faint };
            g.row(Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h), p.label, &value, ink, i == self.list.focus);
        }
        self.list.scrollbar(g, places.len(), top, bottom);
        let hint = match places.get(self.list.focus) {
            Some(p) if p.robot() => format!("{} · enter: use this robot", p.host),
            Some(p) => format!("{} · r: again · t: team", p.host),
            None => String::new(),
        };
        g.hint(&hint);
    }
}

// ------------------------------------------------------------------ ping ---

#[derive(Clone, Default)]
struct Pings {
    /// Each ping in turn: its time, or none for no reply.
    replies: Vec<Option<f32>>,
    /// A word on why none are coming, when that is known.
    note: Option<String>,
}

/// Ping a host once a second, for as long as the page is open.
pub struct Ping {
    host: String,
    field: Option<Field>,
    job: Option<Job<Pings>>,
    seen: u64,
}

impl Ping {
    fn new(robot: Option<String>) -> Self {
        let mut ping = Ping { host: String::new(), field: None, job: None, seen: u64::MAX };
        match robot {
            Some(host) => ping.start(host),
            None => ping.field = Some(Field::new("host or address to ping", "", 64)),
        }
        ping
    }

    fn start(&mut self, host: String) {
        self.host = host.clone();
        self.job = Some(Job::start(Pings::default(), move |report| {
            while !report.stopped() {
                let began = Instant::now();
                match ping_once(&host) {
                    Ok(reply) => report.update(|p| {
                        p.replies.push(reply);
                        if p.replies.len() > 600 {
                            p.replies.remove(0);
                        }
                    }),
                    Err(why) => {
                        report.update(|p| p.note = Some(why));
                        return;
                    }
                }
                report.wait(Duration::from_secs(1).saturating_sub(began.elapsed()));
            }
        }));
    }
}

/// Sent, received and the times, in a line: `12 sent · 0% lost · 0.8 ms avg`.
fn ping_summary(replies: &[Option<f32>]) -> String {
    let times: Vec<f32> = replies.iter().flatten().copied().collect();
    let lost = 100.0 * (replies.len() - times.len()) as f32 / replies.len().max(1) as f32;
    if times.is_empty() {
        return format!("{} sent · {lost:.0}% lost", replies.len());
    }
    let avg = times.iter().sum::<f32>() / times.len() as f32;
    let max = times.iter().cloned().fold(0.0, f32::max);
    format!("{} sent · {lost:.0}% lost · avg {avg:.1} · max {max:.1} ms", replies.len())
}

impl Screen for Ping {
    fn title(&self) -> &str {
        "Ping"
    }

    fn wants_text(&self) -> bool {
        self.field.is_some()
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        if let Some(field) = &mut self.field {
            match field.key(key, host_char) {
                Typed::Done(host) if !host.is_empty() => {
                    self.field = None;
                    self.start(host);
                }
                Typed::Cancelled if self.host.is_empty() => return Nav::Pop,
                Typed::Cancelled => self.field = None,
                _ => {}
            }
            return Nav::Stay;
        }
        match key {
            Key::Back => Nav::Pop,
            Key::Enter => {
                self.field = Some(Field::new("host or address to ping", &self.host, 64));
                Nav::Stay
            }
            _ => Nav::Stay,
        }
    }

    fn tick(&mut self, _dt: f32) -> bool {
        let generation = self.job.as_ref().map_or(0, Job::generation);
        generation != std::mem::replace(&mut self.seen, generation)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        if let Some(field) = &self.field {
            return field.draw(g, "enter: ping it · esc: back");
        }
        let pings = self.job.as_ref().map(Job::get).unwrap_or_default();
        let top = g.top();
        // The host, and the last reply large.
        let head = Rect::new(PAD, top, W - 2.0 * PAD, ty.caption.size + 22.0 * ty.display_scale + 12.0);
        g.surface(head, g.tile_radius(), t.surface1);
        g.text_fit(head.x + 9.0, head.y + 5.0 + ty.caption.size * 0.5, &self.host, ty.caption, t.dim, head.w * 0.6);
        let display = ty.display(22.0);
        let baseline = head.bottom() - 8.0;
        match pings.replies.last() {
            Some(Some(ms)) => {
                let w = g.text(head.x + 8.0, baseline, &format!("{ms:.1}"), display, t.ink);
                g.text(head.x + 10.0 + w, baseline, "ms", ty.caption, t.dim);
            }
            Some(None) => {
                g.text(head.x + 8.0, baseline, "no reply", display, g.fault());
            }
            None => {
                g.text(head.x + 8.0, baseline, "–", display, t.faint);
            }
        }
        let summary = match &pings.note {
            Some(note) => note.clone(),
            None if pings.replies.is_empty() => "pinging…".into(),
            None => ping_summary(&pings.replies),
        };
        // The replies as bars: the last forty, the slow ones taller, a lost one a red mark.
        let plot = Rect::new(PAD, head.bottom() + 5.0, W - 2.0 * PAD, g.above_hint() - head.bottom() - 5.0);
        g.surface(plot, g.tile_radius(), t.surface1);
        let inner = Rect::new(plot.x + 9.0, plot.y + 8.0 + ty.caption.size, plot.w - 18.0, plot.h - 14.0 - ty.caption.size);
        g.text_fit(plot.x + 9.0, plot.y + 5.0 + ty.caption.size * 0.5, &summary, ty.caption, t.dim, plot.w - 18.0);
        const BARS: usize = 40;
        let shown = &pings.replies[pings.replies.len().saturating_sub(BARS)..];
        let peak = shown.iter().flatten().cloned().fold(2.0, f32::max);
        let pitch = inner.w / BARS as f32;
        for (i, reply) in shown.iter().enumerate() {
            let x = inner.x + (BARS - shown.len() + i) as f32 * pitch;
            match reply {
                Some(ms) => {
                    let h = (ms / peak * inner.h).max(2.0);
                    g.surface(Rect::new(x, inner.bottom() - h, (pitch - 2.0).max(1.0), h), 1.0, t.ice);
                }
                None => {
                    let fault = g.fault();
                    g.surface(Rect::new(x, inner.bottom() - 3.0, (pitch - 2.0).max(1.0), 3.0), 1.0, fault);
                }
            }
        }
        g.hint("enter: another host · esc: back");
    }
}

// ----------------------------------------------------------------- ports ---

/// Which of a list of ports a host answers on.
pub struct Ports {
    host: String,
    spec: String,
    field: Option<(Field, bool)>,
    job: Option<Job<Vec<(u16, Port)>>>,
    seen: u64,
    list: ListState,
}

impl Ports {
    fn new(robot: Option<String>) -> Self {
        let mut ports = Ports { host: String::new(), spec: ROBOT_PORTS.into(), field: None, job: None, seen: u64::MAX, list: ListState::default() };
        match robot {
            Some(host) => ports.scan(host),
            None => ports.field = Some((Field::new("host or address to check", "", 64), true)),
        }
        ports
    }

    fn scan(&mut self, host: String) {
        self.host = host.clone();
        let ports = parse_ports(&self.spec).unwrap_or_default();
        let start: Vec<(u16, Port)> = ports.iter().map(|p| (*p, Port::Asking)).collect();
        self.list.focus = 0;
        self.job = Some(Job::start(start, move |report| {
            let Ok(ip) = resolve(&host).map(|a| a[0]) else {
                report.update(|found| found.iter_mut().for_each(|(_, state)| *state = Port::Silent));
                return;
            };
            let items: Vec<(usize, u16)> = ports.iter().copied().enumerate().collect();
            let report2 = report.clone();
            each(items, 12, move |(i, port)| {
                if report2.stopped() {
                    return;
                }
                let state = knock(SocketAddr::new(ip, port));
                report2.update(|found| found[i].1 = state);
            });
        }));
    }
}

impl Screen for Ports {
    fn title(&self) -> &str {
        "Ports"
    }

    fn wants_text(&self) -> bool {
        self.field.is_some()
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some((field, is_host)) = &mut self.field {
            let is_host = *is_host;
            let allowed = |c: char| if is_host { host_char(c) } else { c.is_ascii_digit() || " ,-".contains(c) };
            match field.key(key, allowed) {
                Typed::Done(text) if is_host && !text.is_empty() => {
                    self.field = None;
                    self.scan(text);
                }
                Typed::Done(text) if !is_host => match parse_ports(&text) {
                    Ok(_) => {
                        self.spec = text;
                        self.field = None;
                        let host = self.host.clone();
                        if !host.is_empty() {
                            self.scan(host);
                        }
                    }
                    Err(why) => *cx.say = Some(why),
                },
                Typed::Cancelled if self.host.is_empty() => return Nav::Pop,
                Typed::Cancelled => self.field = None,
                _ => {}
            }
            return Nav::Stay;
        }
        let len = self.job.as_ref().map_or(0, |j| j.get().len());
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter => self.field = Some((Field::new("host or address to check", &self.host, 64), true)),
            Key::Char('p') => self.field = Some((Field::new("ports, as 22 80 5800-5810", &self.spec, 120), false)),
            Key::Char('r') => {
                let host = self.host.clone();
                self.scan(host);
            }
            other => crate::screens::list_key(&mut self.list, other, len),
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        let generation = self.job.as_ref().map_or(0, Job::generation);
        generation != std::mem::replace(&mut self.seen, generation)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        if let Some((field, is_host)) = &self.field {
            return field.draw(g, if *is_host { "enter: check it · esc: back" } else { "enter: check these · esc: back" });
        }
        let found = self.job.as_ref().map(Job::get).unwrap_or_default();
        let top = g.top();
        let open = found.iter().filter(|(_, s)| matches!(s, Port::Open(_))).count();
        let asking = found.iter().any(|(_, s)| *s == Port::Asking);
        let line = format!("{} · {}", self.host, if asking { "checking…".to_owned() } else { format!("{open} of {} open", found.len()) });
        let head_h = ty.caption.size + 6.0;
        g.text_fit(PAD + 4.0, top + head_h * 0.5 - 1.0, &line, ty.caption, t.dim, W - 2.0 * PAD - 8.0);
        let (list_top, row_h, bottom) = (top + head_h + 2.0, g.row_h(), g.above_hint());
        for (i, y) in self.list.layout(g, found.len(), list_top, row_h, 3.0, bottom) {
            let (port, state) = found[i];
            let name = match port_name(port) {
                Some(n) => format!("{port} · {n}"),
                None => port.to_string(),
            };
            let (value, ink) = match state {
                Port::Asking => ("…".to_owned(), t.faint),
                Port::Open(ms) => (format!("open · {ms:.0} ms"), t.ice),
                Port::Closed => ("closed".to_owned(), t.dim),
                Port::Silent => ("no answer".to_owned(), t.faint),
            };
            g.row(Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h), &name, &value, ink, i == self.list.focus);
        }
        self.list.scrollbar(g, found.len(), list_top, bottom);
        g.hint("enter: host · p: ports · r: again");
    }
}

// ---------------------------------------------------------------- lookup ---

#[derive(Clone, Default)]
struct Answer {
    done: bool,
    addresses: Vec<IpAddr>,
    took: f32,
    error: Option<String>,
}

/// A name to its addresses.
pub struct Lookup {
    name: String,
    field: Option<Field>,
    job: Option<Job<Answer>>,
    seen: u64,
    list: ListState,
}

impl Lookup {
    fn new(team: Option<u32>) -> Self {
        let start = team.map_or_else(|| "github.com".to_owned(), |t| format!("roborio-{t}-frc.local"));
        Lookup { name: String::new(), field: Some(Field::new("name to look up", &start, 96)), job: None, seen: u64::MAX, list: ListState::default() }
    }

    fn look(&mut self, name: String) {
        self.name = name.clone();
        self.list.focus = 0;
        self.job = Some(Job::start(Answer::default(), move |report| {
            let began = Instant::now();
            let found = resolve(&name);
            let took = began.elapsed().as_secs_f32() * 1000.0;
            report.update(|a| {
                a.done = true;
                a.took = took;
                match found {
                    Ok(addresses) => a.addresses = addresses,
                    Err(why) => a.error = Some(why),
                }
            });
        }));
    }
}

impl Screen for Lookup {
    fn title(&self) -> &str {
        "Name lookup"
    }

    fn wants_text(&self) -> bool {
        self.field.is_some()
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        if let Some(field) = &mut self.field {
            match field.key(key, host_char) {
                Typed::Done(name) if !name.is_empty() => {
                    self.field = None;
                    self.look(name);
                }
                Typed::Cancelled if self.name.is_empty() => return Nav::Pop,
                Typed::Cancelled => self.field = None,
                _ => {}
            }
            return Nav::Stay;
        }
        let len = self.job.as_ref().map_or(0, |j| j.get().addresses.len());
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter => self.field = Some(Field::new("name to look up", &self.name, 96)),
            other => crate::screens::list_key(&mut self.list, other, len),
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        let generation = self.job.as_ref().map_or(0, Job::generation);
        generation != std::mem::replace(&mut self.seen, generation)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        if let Some(field) = &self.field {
            return field.draw(g, "enter: look it up · esc: back");
        }
        let answer = self.job.as_ref().map(Job::get).unwrap_or_default();
        let top = g.top();
        let head_h = ty.caption.size + 6.0;
        let line = match (&answer.error, answer.done) {
            (_, false) => format!("{} · looking…", self.name),
            (Some(why), true) => why.clone(),
            (None, true) => format!("{} · {:.0} ms", self.name, answer.took),
        };
        g.text_fit(PAD + 4.0, top + head_h * 0.5 - 1.0, &line, ty.caption, t.dim, W - 2.0 * PAD - 8.0);
        let (list_top, row_h, bottom) = (top + head_h + 2.0, g.row_h(), g.above_hint());
        for (i, y) in self.list.layout(g, answer.addresses.len(), list_top, row_h, 3.0, bottom) {
            let ip = answer.addresses[i];
            let kind = if ip.is_ipv4() { "ipv4" } else { "ipv6" };
            g.row(Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h), &ip.to_string(), kind, t.dim, i == self.list.focus);
        }
        self.list.scrollbar(g, answer.addresses.len(), list_top, bottom);
        if answer.done && answer.addresses.is_empty() {
            g.empty("network_check", "nothing found");
        }
        g.hint("enter: another name · esc: back");
    }
}

// ---------------------------------------------------------------- survey ---

/// The networks in range, strongest first, with their signal in dBm.
pub struct Survey {
    wifi: crate::wifi::Wifi,
    list: ListState,
    seen: u64,
    scanned: Instant,
}

impl Survey {
    fn new() -> Self {
        let wifi = crate::wifi::Wifi::open();
        wifi.scan();
        Survey { wifi, list: ListState::default(), seen: u64::MAX, scanned: Instant::now() }
    }

    fn networks(&self) -> Vec<crate::wifi::Network> {
        let mut seen = self.wifi.state().networks;
        seen.sort_by(|a, b| b.signal.cmp(&a.signal).then(a.ssid.cmp(&b.ssid)));
        seen
    }
}

impl Screen for Survey {
    fn title(&self) -> &str {
        "Wi-Fi survey"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter if !self.wifi.state().enabled => {
                self.wifi.enable(true);
                *cx.say = Some("switching wi-fi on".into());
            }
            Key::Char('r') => {
                self.wifi.scan();
                self.scanned = Instant::now();
            }
            other => {
                let len = self.networks().len();
                crate::screens::list_key(&mut self.list, other, len);
            }
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        // A fresh look every eight seconds while the page is open.
        if self.scanned.elapsed() >= Duration::from_secs(8) {
            self.wifi.scan();
            self.scanned = Instant::now();
        }
        let generation = self.wifi.generation();
        generation != std::mem::replace(&mut self.seen, generation)
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let state = self.wifi.state();
        if !state.enabled {
            g.empty("wifi_off", "wi-fi is off · enter switches it on");
            return;
        }
        let networks = self.networks();
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        for (i, y) in self.list.layout(g, networks.len(), top, row_h, 3.0, bottom) {
            let n = &networks[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let name = if n.ssid.is_empty() { "(hidden)".to_owned() } else { n.ssid.clone() };
            let value = format!("{} dbm{}", n.signal, if n.secured { "" } else { " · open" });
            g.row(r, &name, &value, if n.current { t.ice } else { t.dim }, i == self.list.focus);
            // Its bars, just before the reading.
            let value_w = g.measure(&value, if g.theme == crate::look::Theme::Bezel { g.ty.body } else { g.ty.label });
            let right = r.right() - 10.0 - value_w - 8.0;
            for b in 0..4 {
                let h = 3.0 + b as f32 * 2.0;
                let bar = Rect::new(right - (4 - b) as f32 * 4.0, r.cy() + 5.0 - h, 2.5, h);
                g.surface(bar, 1.0, if b < n.bars() { t.ice } else { t.meter });
            }
        }
        self.list.scrollbar(g, networks.len(), top, bottom);
        if networks.is_empty() {
            g.empty("wifi", "looking…");
        }
        g.hint(&format!("{} · r: look again", if state.status.is_empty() { "in range" } else { &state.status }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ping_reply_is_read_from_busybox_and_windows() {
        assert_eq!(parse_ping("PING 10.58.5.2 (10.58.5.2): 56 data bytes\n64 bytes from 10.58.5.2: seq=0 ttl=64 time=0.812 ms\n"), Some(0.812));
        assert_eq!(parse_ping("Reply from 127.0.0.1: bytes=32 time<1ms TTL=128\n"), Some(0.5));
        assert_eq!(parse_ping("Reply from 8.8.8.8: bytes=32 time=14ms TTL=117\n"), Some(14.0));
        assert_eq!(parse_ping("Reply from 10.0.0.1: Destination host unreachable.\n"), None, "no time to live, no reply");
        assert_eq!(parse_ping("1 packets transmitted, 0 packets received, 100% packet loss\n"), None);
    }

    #[test]
    fn ports_are_written_as_a_list() {
        assert_eq!(parse_ports("22, 80 5800-5802").unwrap(), vec![22, 80, 5800, 5801, 5802]);
        assert!(parse_ports("70000").is_err());
        assert!(parse_ports("10-1").is_err());
        assert!(parse_ports("1-9999").is_err(), "a range is kept short");
        assert!(parse_ports(" ").is_err());
        assert_eq!(parse_ports(ROBOT_PORTS).unwrap().len(), 8);
        assert_eq!(port_name(5810), Some("nt4"));
        assert_eq!(port_name(9010), Some("agent"));
    }

    #[test]
    fn the_robot_is_looked_for_where_a_team_puts_it() {
        let places = places(5805);
        assert_eq!(places[0].1, "10.58.5.2");
        assert!(places.iter().any(|(_, h)| h == "roborio-5805-frc.local"));
        assert!(places.iter().any(|(_, h)| h == "172.22.11.2"));
        assert_eq!(places.iter().find(|(l, _)| *l == "Radio").unwrap().1, "10.58.5.1");
        let short = super::places(254);
        assert_eq!(short[0].1, "10.2.54.2");
    }

    #[test]
    fn an_open_port_and_a_closed_one_are_told_apart() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let open = listener.local_addr().unwrap();
        assert!(matches!(knock(open), Port::Open(_)));
        drop(listener);
        // Refused at once on the handheld; Windows tries again for a couple
        // of seconds before it says so, longer than a port is given.
        let closed = knock(open);
        if cfg!(windows) {
            assert!(matches!(closed, Port::Closed | Port::Silent));
        } else {
            assert_eq!(closed, Port::Closed, "nothing listening any more");
        }
        assert_eq!(resolve("127.0.0.1").unwrap(), vec!["127.0.0.1".parse::<IpAddr>().unwrap()]);
        assert!(resolve("localhost").unwrap().iter().any(|ip| ip.is_loopback()));
    }

    #[test]
    fn a_port_check_reports_as_it_goes_and_stops_with_its_page() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut ports = Ports::new(None);
        ports.spec = format!("{port}");
        ports.field = None;
        ports.scan("127.0.0.1".into());
        let until = Instant::now() + Duration::from_secs(5);
        while ports.job.as_ref().unwrap().get().iter().any(|(_, s)| *s == Port::Asking) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(matches!(ports.job.as_ref().unwrap().get()[0].1, Port::Open(_)));
        assert!(ports.tick(0.0), "the page redraws for what came in");
        assert!(!ports.tick(0.0), "and not again for nothing");
    }

    #[test]
    fn a_ping_summary_reads_as_a_line() {
        assert_eq!(ping_summary(&[Some(1.0), None, Some(3.0), Some(2.0)]), "4 sent · 25% lost · avg 2.0 · max 3.0 ms");
        assert_eq!(ping_summary(&[None, None]), "2 sent · 100% lost");
    }
}
