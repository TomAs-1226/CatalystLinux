//! Monitor: how the handheld itself is doing, as it happens. The processor
//! (all of it, and each core), memory, temperature and network traffic, each
//! with the last minute as a small graph; then the processes using the most.
//!
//! Read from `/proc` and `/sys` once a second, and only while the app is
//! open: nothing keeps watching once it is closed. On a computer there is no
//! `/proc` to read, so a simulated handheld stands in, and says so.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use bezel::{Align, Color, Rect};
use detent::{role, Animated};

use crate::gfx::{Gfx, ListState, GAP, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{bytes, sparkline};

/// How often a reading is taken, and how many are kept for the graphs:
/// the last half minute.
const EVERY: Duration = Duration::from_secs(1);
const KEPT: usize = 30;

/// One process, as last read.
#[derive(Clone, Debug, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub name: String,
    /// Its share of the whole processor since the last reading, 0 to 1.
    pub cpu: f32,
    /// Memory it holds (resident), in kB.
    pub mem: u64,
}

/// Everything read at once.
#[derive(Clone, Debug, Default)]
pub struct Reading {
    /// How busy the processor was since the last reading, 0 to 1: all of
    /// it, and each core.
    pub cpu: Option<f32>,
    pub cores: Vec<f32>,
    /// Memory in use and in all, in kB.
    pub mem: Option<(u64, u64)>,
    /// The processor's temperature, °C.
    pub temp: Option<f32>,
    /// Bytes a second in and out, over every interface but loopback.
    pub net: Option<(f64, f64)>,
    pub load: Option<f32>,
    pub uptime: Option<f64>,
    pub procs: Vec<Proc>,
    /// Made up, because this is not a handheld.
    pub simulated: bool,
}

// --------------------------------------------------------------- reading ---

/// Busy and total time for the whole processor, then each core, from
/// `/proc/stat`, in clock ticks.
pub fn cpu_times(stat: &str) -> Vec<(u64, u64)> {
    stat.lines()
        .filter(|l| l.starts_with("cpu"))
        .map(|l| {
            let v: Vec<u64> = l.split_whitespace().skip(1).filter_map(|f| f.parse().ok()).collect();
            // user nice system idle iowait irq softirq steal (guest time is counted in user already).
            let total: u64 = v.iter().take(8).sum();
            let idle = v.get(3).copied().unwrap_or(0) + v.get(4).copied().unwrap_or(0);
            (total - idle.min(total), total)
        })
        .collect()
}

/// How busy, 0 to 1, between two readings of (busy, total).
fn busy(before: (u64, u64), now: (u64, u64)) -> f32 {
    let total = now.1.saturating_sub(before.1);
    if total == 0 {
        return 0.0;
    }
    (now.0.saturating_sub(before.0) as f32 / total as f32).clamp(0.0, 1.0)
}

/// Memory in use and in all, in kB, from `/proc/meminfo`.
pub fn meminfo(text: &str) -> Option<(u64, u64)> {
    let field = |name: &str| -> Option<u64> { text.lines().find(|l| l.starts_with(name))?.split_whitespace().nth(1)?.parse().ok() };
    let (total, available) = (field("MemTotal:")?, field("MemAvailable:")?);
    Some((total.saturating_sub(available), total))
}

/// Bytes received and sent in all, from `/proc/net/dev`, loopback left out.
pub fn net_bytes(text: &str) -> (u64, u64) {
    text.lines()
        .filter_map(|l| l.split_once(':'))
        .filter(|(name, _)| name.trim() != "lo")
        .map(|(_, rest)| {
            let v: Vec<u64> = rest.split_whitespace().filter_map(|f| f.parse().ok()).collect();
            (v.first().copied().unwrap_or(0), v.get(8).copied().unwrap_or(0))
        })
        .fold((0, 0), |(rx, tx), (r, t)| (rx + r, tx + t))
}

/// A process's name, its processor time in clock ticks, and the pages it
/// holds, from `/proc/<pid>/stat`. The name is in brackets and may hold
/// spaces and brackets of its own, so the rest is read from the last one.
pub fn proc_stat(text: &str) -> Option<(String, u64, u64)> {
    let (open, close) = (text.find('(')?, text.rfind(')')?);
    let name = text.get(open + 1..close)?.to_owned();
    let rest: Vec<&str> = text.get(close + 1..)?.split_whitespace().collect();
    // After the name: state is field 3, utime 14, stime 15, rss 24.
    let field = |n: usize| -> Option<u64> { rest.get(n - 3)?.parse().ok() };
    Some((name, field(14)? + field(15)?, field(24)?))
}

/// The handheld, read from `/proc` and `/sys`, against the last reading.
#[derive(Default)]
struct Real {
    cpu: Vec<(u64, u64)>,
    net: Option<(u64, u64, Instant)>,
    procs: HashMap<u32, u64>,
}

impl Real {
    fn read(&mut self) -> Reading {
        let text = |path: &str| std::fs::read_to_string(path).ok();
        let mut r = Reading::default();
        let mut whole = None;
        if let Some(stat) = text("/proc/stat") {
            let now = cpu_times(&stat);
            if now.len() == self.cpu.len() && !now.is_empty() {
                r.cpu = Some(busy(self.cpu[0], now[0]));
                r.cores = self.cpu.iter().zip(&now).skip(1).map(|(b, n)| busy(*b, *n)).collect();
                whole = Some(now[0].1.saturating_sub(self.cpu[0].1));
            }
            self.cpu = now;
        }
        r.mem = text("/proc/meminfo").and_then(|m| meminfo(&m));
        r.temp = text("/sys/class/thermal/thermal_zone0/temp").and_then(|t| t.trim().parse::<f32>().ok()).map(|m| m / 1000.0);
        if let Some(dev) = text("/proc/net/dev") {
            let (rx, tx) = net_bytes(&dev);
            let at = Instant::now();
            if let Some((rx0, tx0, then)) = self.net {
                let s = at.duration_since(then).as_secs_f64().max(0.001);
                r.net = Some((rx.saturating_sub(rx0) as f64 / s, tx.saturating_sub(tx0) as f64 / s));
            }
            self.net = Some((rx, tx, at));
        }
        r.load = text("/proc/loadavg").and_then(|l| l.split_whitespace().next()?.parse().ok());
        r.uptime = text("/proc/uptime").and_then(|u| u.split_whitespace().next()?.parse().ok());
        // Processes: what each used since the last reading. Kernel threads
        // hold no memory of their own and are left out.
        let mut seen = HashMap::new();
        if let Ok(dir) = std::fs::read_dir("/proc") {
            for entry in dir.flatten() {
                let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
                let Some((name, ticks, pages)) = text(&format!("/proc/{pid}/stat")).and_then(|s| proc_stat(&s)) else { continue };
                seen.insert(pid, ticks);
                if pages == 0 {
                    continue;
                }
                let cpu = match (whole, self.procs.get(&pid)) {
                    (Some(all), Some(before)) if all > 0 => ticks.saturating_sub(*before) as f32 / all as f32,
                    _ => 0.0,
                };
                r.procs.push(Proc { pid, name, cpu, mem: pages * 4 });
            }
        }
        self.procs = seen;
        r
    }
}

/// A handheld made up for a computer: steady enough to read, moving
/// enough to show what the screen does.
struct Simulated {
    started: Instant,
}

impl Simulated {
    fn read(&mut self) -> Reading {
        let t = self.started.elapsed().as_secs_f32();
        // A little noise that is the same every run.
        let wobble = |k: f32| ((t * 1.7 + k).sin() * 43758.5).fract().abs();
        let cpu = (0.10 + 0.06 * (t * 0.45).sin() + 0.05 * wobble(1.0)).clamp(0.0, 1.0);
        let cores: Vec<f32> = (0..4).map(|c| (cpu + 0.08 * ((t * 0.7 + c as f32 * 1.9).sin()) + 0.04 * wobble(c as f32)).clamp(0.0, 1.0)).collect();
        let procs = [
            ("catalyst-shell", 0.55, 6_150),
            ("wpa_supplicant", 0.05, 2_380),
            ("ntpd", 0.01, 640),
            ("udhcpc", 0.01, 520),
            ("syslogd", 0.02, 512),
            ("klogd", 0.0, 470),
            ("init", 0.0, 410),
        ];
        Reading {
            cpu: Some(cpu),
            cores,
            mem: Some((25_800 + (900.0 * (t * 0.2).sin()) as u64, 427_000)),
            temp: Some(47.5 + 2.5 * (t * 0.11).sin() + 0.3 * wobble(5.0)),
            net: Some((1_800.0 + 1_600.0 * (t * 0.33).sin().abs() as f64, 520.0 + 240.0 * wobble(7.0) as f64)),
            load: Some(0.16 + 0.06 * (t * 0.05).sin()),
            uptime: Some(3.0 * 3600.0 + 754.0 + t as f64),
            procs: procs
                .iter()
                .enumerate()
                .map(|(i, (name, share, mem))| Proc { pid: 1 + i as u32 * 97, name: (*name).into(), cpu: cpu * share * (0.8 + 0.4 * wobble(i as f32)), mem: *mem })
                .collect(),
            simulated: true,
        }
    }
}

enum Source {
    Real(Real),
    Simulated(Simulated),
}

impl Source {
    fn open() -> Source {
        if std::path::Path::new("/proc/stat").exists() {
            Source::Real(Real::default())
        } else {
            Source::Simulated(Simulated { started: Instant::now() })
        }
    }

    fn read(&mut self) -> Reading {
        match self {
            Source::Real(real) => real.read(),
            Source::Simulated(sim) => sim.read(),
        }
    }
}

/// How long it has been up, in a few characters: `3 h 12 min`.
pub fn uptime(seconds: f64) -> String {
    let minutes = (seconds / 60.0) as u64;
    match (minutes / 1440, minutes / 60 % 24, minutes % 60) {
        (0, 0, m) => format!("{m} min"),
        (0, h, m) => format!("{h} h {m} min"),
        (d, h, _) => format!("{d} d {h} h"),
    }
}

// ---------------------------------------------------------------- screen ---

pub struct Monitor {
    source: Source,
    read_at: Option<Instant>,
    now: Reading,
    cpu: VecDeque<f32>,
    mem: VecDeque<f32>,
    temp: VecDeque<f32>,
    net: VecDeque<f32>,
    /// Which page: 0 the overview, 1 the processes; and where the turn is.
    page: usize,
    turn: Animated,
    by_memory: bool,
    list: ListState,
}

impl Monitor {
    pub fn new() -> Self {
        let mut monitor = Monitor {
            source: Source::open(),
            read_at: None,
            now: Reading::default(),
            cpu: VecDeque::new(),
            mem: VecDeque::new(),
            temp: VecDeque::new(),
            net: VecDeque::new(),
            page: 0,
            turn: Animated::new(0.0, role::page()).with_rest(0.002),
            by_memory: false,
            list: ListState::default(),
        };
        monitor.sample();
        monitor
    }

    fn sample(&mut self) {
        self.read_at = Some(Instant::now());
        self.now = self.source.read();
        let push = |kept: &mut VecDeque<f32>, v: Option<f32>| {
            if let Some(v) = v {
                kept.push_back(v);
                if kept.len() > KEPT {
                    kept.pop_front();
                }
            }
        };
        let r = &self.now;
        push(&mut self.cpu, r.cpu);
        push(&mut self.mem, r.mem.map(|(used, all)| used as f32 / all.max(1) as f32));
        push(&mut self.temp, r.temp);
        push(&mut self.net, r.net.map(|(rx, tx)| (rx + tx) as f32));
    }

    /// The processes, busiest first (or the largest, sorted by memory).
    fn processes(&self) -> Vec<Proc> {
        let mut procs = self.now.procs.clone();
        if self.by_memory {
            procs.sort_by(|a, b| b.mem.cmp(&a.mem).then(a.pid.cmp(&b.pid)));
        } else {
            procs.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.mem.cmp(&a.mem)));
        }
        procs
    }

    fn draw_overview(&self, g: &mut Gfx) {
        let t = g.tokens;
        let top = g.top();
        let card_w = (W - 2.0 * PAD - GAP) / 2.0;
        let card_h = ((H - 16.0 - top - GAP) / 2.0).floor();
        let at = |i: usize| Rect::new(PAD + (i % 2) as f32 * (card_w + GAP), top + (i / 2) as f32 * (card_h + GAP), card_w, card_h);
        let r = &self.now;
        let hot = |c: f32| if c >= 80.0 { g.fault() } else if c >= 70.0 { t.amber_ink } else { t.ice };
        let cpu_ink = if r.cpu.is_some_and(|c| c > 0.85) { t.signal } else { t.ice };
        let temp_ink = r.temp.map_or(t.ice, hot);
        let load = r.load.map(|l| format!("load {l:.2}"));
        card(g, at(0), "cpu", load, r.cpu.map(|c| (format!("{:.0}", c * 100.0), "%".into())), &self.cpu, 1.0, cpu_ink);
        cores(g, at(0), &r.cores);
        let mem = r.mem.map(|(used, _)| (format!("{:.0}", used as f32 / 1024.0), "mb".into()));
        card(g, at(1), "memory", r.mem.map(|(_, all)| format!("of {:.0} mb", all as f32 / 1024.0)), mem, &self.mem, 1.0, t.ice);
        let up = r.uptime.map(|u| format!("up {}", uptime(u)));
        card(g, at(2), "temp", up, r.temp.map(|c| (format!("{c:.1}"), "°c".into())), &self.temp, 85.0, temp_ink);
        let split = |n: f64| {
            let s = bytes(n);
            let (n, unit) = s.split_once(' ').unwrap_or((&s, ""));
            (n.to_owned(), format!("{unit}/s"))
        };
        let sent = r.net.map(|(_, tx)| format!("↑ {}/s", bytes(tx)));
        card(g, at(3), "net ↓", sent, r.net.map(|(rx, _)| split(rx)), &self.net, 4096.0, t.ice);
    }

    fn draw_processes(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let procs = self.processes();
        let head_h = ty.caption.size + 6.0;
        let sort = if self.by_memory { "by memory" } else { "by cpu" };
        let left = format!("{}{} · {} running", if self.now.simulated { "simulated · " } else { "" }, sort, procs.len());
        g.text_fit(PAD + 4.0, top + head_h * 0.5 - 1.0, &left, ty.caption, t.dim, W - 2.0 * PAD - 8.0);
        let (list_top, row_h, bottom) = (top + head_h + 2.0, g.row_h(), H - 16.0);
        for (i, y) in self.list.layout(g, procs.len(), list_top, row_h, 3.0, bottom) {
            let p = &procs[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let value = format!("{:>4.1}% · {}", p.cpu * 100.0, bytes(p.mem as f64 * 1024.0));
            g.row(r, &p.name, &value, if p.cpu >= 0.05 { t.ink } else { t.dim }, i == self.list.focus);
        }
        self.list.scrollbar(g, procs.len(), list_top, bottom - 2.0);
        if procs.is_empty() {
            g.text_at(W * 0.5, (list_top + bottom) * 0.5, Align::Center, "reading…", ty.caption, t.dim);
        }
    }
}

/// A card: its name, and beside it a second reading; the value large with
/// its unit; and the last half minute as a graph filling its right half.
#[allow(clippy::too_many_arguments)]
fn card(g: &mut Gfx, r: Rect, label: &str, aside: Option<String>, value: Option<(String, String)>, history: &VecDeque<f32>, max: f32, ink: Color) {
    let t = g.tokens;
    let ty = g.ty;
    g.surface(r, g.tile_radius(), t.surface1);
    let cy = r.y + 5.0 + ty.caption.size * 0.5;
    let label_w = g.text_at(r.x + 9.0, cy, Align::Left, label, ty.caption, t.dim);
    if let Some(aside) = aside {
        let room = r.w - 18.0 - label_w - 8.0;
        let fitted = g.text.fit(&aside, &ty.caption, room);
        g.text_at(r.right() - 9.0, cy, Align::Right, &fitted, ty.caption, t.dim);
    }
    let display = ty.display(20.0);
    let baseline = r.y + 9.0 + ty.caption.size + display.size * 0.82;
    match &value {
        Some((v, unit)) => {
            let w = g.text(r.x + 8.0, baseline, v, display, t.ink);
            g.text(r.x + 11.0 + w, baseline, unit, ty.caption, t.dim);
        }
        None => {
            g.text(r.x + 8.0, baseline, "–", display, t.faint);
        }
    }
    let graph_x = r.x + r.w * 0.5;
    let graph = Rect::new(graph_x, r.y + 12.0 + ty.caption.size, r.right() - 9.0 - graph_x, r.h - 18.0 - ty.caption.size);
    let values: Vec<f32> = history.iter().copied().collect();
    sparkline(g, graph, &values, max, ink, KEPT);
}

/// Each core, a short bar along the bottom left of the processor's card.
fn cores(g: &mut Gfx, r: Rect, cores: &[f32]) {
    if cores.is_empty() {
        return;
    }
    let t = g.tokens;
    let w = ((r.w * 0.5 - 14.0 - 3.0 * (cores.len() - 1) as f32) / cores.len() as f32).clamp(6.0, 16.0);
    let y = r.bottom() - 10.0;
    for (i, c) in cores.iter().enumerate() {
        let bar = Rect::new(r.x + 9.0 + i as f32 * (w + 3.0), y, w, 4.0);
        g.surface(bar, 2.0, t.meter);
        if *c > 0.01 {
            g.surface(Rect::new(bar.x, bar.y, (bar.w * c).max(3.0), bar.h), 2.0, if *c > 0.85 { t.signal } else { t.ice });
        }
    }
}

impl Screen for Monitor {
    fn title(&self) -> &str {
        "Monitor"
    }

    fn greeting(&self) -> Option<&'static str> {
        self.now.simulated.then_some("simulated on this computer")
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Back => return Nav::Pop,
            Key::Right if self.page == 0 => self.page = 1,
            Key::Enter if self.page == 0 => self.page = 1,
            Key::Left if self.page == 1 => self.page = 0,
            Key::Enter => {
                self.by_memory = !self.by_memory;
                self.list.focus = 0;
            }
            other if self.page == 1 => crate::screens::list_key(&mut self.list, other, self.now.procs.len()),
            _ => {}
        }
        if self.turn.target() != self.page as f32 {
            self.turn.set(self.page as f32);
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        let moving = self.turn.tick(dt);
        // By the clock, not the motion: it reads on even when nothing moves.
        if self.read_at.is_none_or(|at| at.elapsed() >= EVERY) {
            self.sample();
            return true;
        }
        moving
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        if g.calm {
            self.turn.jump(self.page as f32);
        }
        let base = g.ox;
        let at = self.turn.value();
        for page in 0..2 {
            let ox = (page as f32 - at) * W;
            if ox.abs() >= W {
                continue;
            }
            g.ox = base + ox.round();
            if page == 0 {
                self.draw_overview(g);
            } else {
                self.draw_processes(g);
            }
        }
        g.ox = base;
        g.pager = Some((2, self.page, 0.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_processor_is_read_from_proc_stat() {
        let before = cpu_times("cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 50 0 25 400 25 0 0 0\ncpu1 50 0 25 400 25 0 0 0\nintr 1 2 3\n");
        let now = cpu_times("cpu  160 0 70 860 60 0 0 0 0 0\ncpu0 110 0 45 410 25 0 0 0\ncpu1 50 0 25 450 35 0 0 0\n");
        assert_eq!(before.len(), 3, "the whole, then each core");
        assert!((busy(before[0], now[0]) - 80.0 / 150.0).abs() < 1e-4);
        assert!((busy(before[1], now[1]) - 80.0 / 90.0).abs() < 1e-4, "core 0 busy");
        assert_eq!(busy(before[2], now[2]), 0.0, "core 1 idle");
    }

    #[test]
    fn memory_network_and_processes_are_read_as_the_kernel_writes_them() {
        assert_eq!(meminfo("MemTotal:  437248 kB\nMemFree: 300000 kB\nMemAvailable:  390000 kB\n"), Some((47_248, 437_248)));
        let dev = "Inter-|   Receive\n face |bytes packets\n    lo: 9999 10 0 0 0 0 0 0 9999 10 0 0 0 0 0 0\n  eth0: 1200 8 0 0 0 0 0 0 300 4 0 0 0 0 0 0\n wlan0: 50 1 0 0 0 0 0 0 20 1 0 0 0 0 0 0\n";
        assert_eq!(net_bytes(dev), (1250, 320), "loopback left out");
        // A name with spaces and brackets in it.
        let stat = "1234 (my (odd) prog) S 1 1234 1234 0 -1 4194304 100 0 0 0 250 50 0 0 20 0 1 0 500 9000000 1500 18446744073709551615";
        assert_eq!(proc_stat(stat), Some(("my (odd) prog".into(), 300, 1500)));
    }

    #[test]
    fn uptime_reads_as_people_say_it() {
        assert_eq!(uptime(59.0 * 60.0), "59 min");
        assert_eq!(uptime(3.0 * 3600.0 + 12.0 * 60.0), "3 h 12 min");
        assert_eq!(uptime(2.0 * 86_400.0 + 5.0 * 3600.0), "2 d 5 h");
    }

    #[test]
    fn a_reading_comes_once_a_second_and_the_graphs_keep_a_minute() {
        let mut monitor = Monitor::new();
        assert_eq!(monitor.mem.len(), 1, "memory is there from the first reading");
        assert!(!monitor.tick(0.0), "not again straight away");
        for _ in 0..KEPT + 5 {
            monitor.read_at = Some(Instant::now() - EVERY);
            assert!(monitor.tick(0.0));
        }
        assert_eq!(monitor.mem.len(), KEPT);
        assert!(monitor.now.procs.iter().any(|p| p.mem > 0));
        // Busiest first, or largest.
        let busiest = monitor.processes();
        assert!(busiest.windows(2).all(|w| w[0].cpu >= w[1].cpu));
        monitor.by_memory = true;
        let largest = monitor.processes();
        assert!(largest.windows(2).all(|w| w[0].mem >= w[1].mem));
    }
}
