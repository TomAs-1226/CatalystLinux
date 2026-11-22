//! `catalyst-shell --sched-bench [clock|power|io]`: the handheld's own
//! figures for the choices in sched.rs, as text. Run it with the shell's
//! service stopped (`rc-service catalyst-shell stop`), so nothing else moves
//! the clock or the light meanwhile; the clock's limits are put back as
//! they were when it ends (and a shell starting puts the floor back too).
//!
//! - **clock**: the governor and its tunables; how long a raised floor
//!   takes to become the clock (the boost's own delay); what a frame costs
//!   at the least clock and at the top.
//! - **power**: from the fuel gauge, on battery: the draw at rest at each
//!   clock, and drawing frames without a pause at the least and the top,
//!   so what a frame costs in energy at each, and what the top costs at rest
//!   (the firmware's turbo raises the core and memory clocks with it). 20 s
//!   a point, about three minutes in all.
//! - **io**: the card's scheduler and its tunables, and how long a settings
//!   save (a small file written and flushed, as the drawing thread does) and
//!   an uncached 4 KB read take: quiet, behind a song being saved (nice 10,
//!   the card's idle class) and behind an update being written (SCHED_IDLE,
//!   flushed every 4 MB).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::platform::Key;
use crate::shell::Shell;

pub fn run(what: &str, shell: impl FnOnce() -> Shell) {
    match what {
        "clock" => clock(shell()),
        "power" => power(shell()),
        "io" => io(),
        other => eprintln!("catalyst-shell --sched-bench clock|power|io (not {other})"),
    }
}

/// p50, p95 and the worst of some milliseconds.
fn spread(mut v: Vec<f64>) -> String {
    if v.is_empty() {
        return "–".into();
    }
    v.sort_by(f64::total_cmp);
    let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    format!("p50 {:.2} ms · p95 {:.2} ms · worst {:.2} ms ({} tries)", at(0.5), at(0.95), v[v.len() - 1], v.len())
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_owned())
}

// ----------------------------------------------------------------- clock ---

/// The clock's limits (the first policy: the CM0 has one), and how to pin
/// it at one speed and give it back.
struct Limits {
    dir: PathBuf,
    min: String,
    max: String,
    least: u32,
    top: u32,
    steps: Vec<u32>,
}

impl Limits {
    fn read() -> Option<Limits> {
        let dir = PathBuf::from("/sys/devices/system/cpu/cpufreq/policy0");
        let num = |n: &str| read(&dir.join(n))?.parse::<u32>().ok();
        let mut steps: Vec<u32> = read(&dir.join("scaling_available_frequencies")).unwrap_or_default().split_whitespace().filter_map(|f| f.parse().ok()).collect();
        steps.sort_unstable();
        Some(Limits { min: read(&dir.join("scaling_min_freq"))?, max: read(&dir.join("scaling_max_freq"))?, least: num("cpuinfo_min_freq")?, top: num("cpuinfo_max_freq")?, steps, dir })
    }

    fn write(&self, name: &str, khz: u32) {
        std::fs::write(self.dir.join(name), khz.to_string()).ok();
    }

    /// Held at `khz`, whatever the governor.
    fn pin(&self, khz: u32) {
        self.write("scaling_min_freq", self.least);
        self.write("scaling_max_freq", khz);
        self.write("scaling_min_freq", khz);
        self.settle_at(khz, Duration::from_millis(500));
    }

    fn cur(&self) -> u32 {
        read(&self.dir.join("scaling_cur_freq")).and_then(|s| s.parse().ok()).unwrap_or(0)
    }

    /// Wait until the clock reads `khz`, or `within` passes. How long it took.
    fn settle_at(&self, khz: u32, within: Duration) -> Option<Duration> {
        let began = Instant::now();
        while began.elapsed() < within {
            if self.cur() == khz {
                return Some(began.elapsed());
            }
            std::thread::sleep(Duration::from_micros(100));
        }
        None
    }

    fn restore(&self) {
        std::fs::write(self.dir.join("scaling_min_freq"), &self.least.to_string()).ok();
        std::fs::write(self.dir.join("scaling_max_freq"), &self.max).ok();
        std::fs::write(self.dir.join("scaling_min_freq"), &self.min).ok();
    }
}

/// What drawing a frame costs here: a page turning, an app opening and
/// closing, as `--bench` draws them. Milliseconds a frame.
fn frame_ms(shell: &mut Shell, frames: u32) -> f64 {
    let keys = [Key::Enter, Key::Back, Key::Right, Key::Enter, Key::Down, Key::Back, Key::Right, Key::Enter, Key::Down, Key::Enter, Key::Back, Key::Back];
    let began = Instant::now();
    for i in 0..frames {
        if i % 12 == 0 {
            shell.key(keys[(i / 12) as usize % keys.len()]);
        }
        shell.tick(1.0 / 40.0);
        shell.draw();
    }
    ms(began.elapsed()) / frames.max(1) as f64
}

fn clock(mut shell: Shell) {
    let Some(limits) = Limits::read() else {
        println!("no cpufreq here: the clock is the handheld's to measure");
        return;
    };
    let cpu = Path::new("/sys/devices/system/cpu");
    let governor = read(&limits.dir.join("scaling_governor")).unwrap_or_default();
    println!("governor        {governor} (of {})", read(&limits.dir.join("scaling_available_governors")).unwrap_or_default());
    println!("steps           {:?} kHz, now {} (floor {}, ceiling {})", limits.steps, limits.cur(), limits.min, limits.max);
    println!("latency         {} ns (cpuinfo_transition_latency)", read(&limits.dir.join("cpuinfo_transition_latency")).unwrap_or_else(|| "–".into()));
    for (name, file) in [("schedutil", "schedutil/rate_limit_us"), ("ondemand", "ondemand/sampling_rate"), ("ondemand", "ondemand/up_threshold"), ("conservative", "conservative/ignore_nice_load")] {
        for dir in [cpu.join("cpufreq"), limits.dir.clone()] {
            if let Some(v) = read(&dir.join(file)) {
                println!("{name:<15} {} = {v}", file.split('/').nth(1).unwrap_or(file));
            }
        }
    }
    println!("cpuidle         {}", read(&cpu.join("cpuidle/current_driver")).unwrap_or_else(|| "none (idle is WFI)".into()));
    println!("transitions     {} so far", read(&limits.dir.join("stats/total_trans")).unwrap_or_else(|| "– (no cpufreq stats)".into()));

    // The boost's own delay: the floor raised from rest, as sched.rs's thread does it.
    let (least, top) = (limits.least, limits.top);
    let (mut writes, mut landed, mut missed) = (Vec::new(), Vec::new(), 0);
    limits.write("scaling_max_freq", top);
    for _ in 0..20 {
        limits.write("scaling_min_freq", least);
        if limits.settle_at(least, Duration::from_millis(800)).is_none() {
            missed += 1;
            continue;
        }
        std::thread::sleep(Duration::from_millis(50));
        let began = Instant::now();
        limits.write("scaling_min_freq", top);
        writes.push(ms(began.elapsed()));
        match limits.settle_at(top, Duration::from_millis(100)) {
            Some(_) => landed.push(ms(began.elapsed())),
            None => missed += 1,
        }
    }
    limits.write("scaling_min_freq", least);
    println!("boost: write    {}", spread(writes));
    println!("boost: landed   {}{}", spread(landed), if missed > 0 { format!(" · {missed} never came down or up in time") } else { String::new() });

    // What a frame costs at the least clock and the top.
    let mut costs = Vec::new();
    for khz in [least, top] {
        limits.pin(khz);
        frame_ms(&mut shell, 36);
        let cost = frame_ms(&mut shell, 240);
        println!("frame at {:>4} MHz: {cost:.2} ms", khz / 1000);
        costs.push(cost);
    }
    if let [slow, fast] = costs[..] {
        println!("frame: {:.2}× as long at {} MHz ({:.2}× if it were all processor)", slow / fast, least / 1000, top as f64 / least as f64);
    }
    limits.restore();
}

// ----------------------------------------------------------------- power ---

fn power(mut shell: Shell) {
    let Some(limits) = Limits::read() else {
        println!("no cpufreq here");
        return;
    };
    match crate::power::Reading::read() {
        Some(r) if r.flow == crate::power::Flow::Discharging => {}
        Some(_) => {
            println!("on the charger: unplug it, the gauge reads what the battery gives");
            return;
        }
        None => {
            println!("no fuel gauge here");
            return;
        }
    }
    let mut middle: Vec<u32> = limits.steps.iter().copied().filter(|s| *s > limits.least && *s < limits.top).collect();
    middle.truncate(1);
    let mut rest_points = vec![limits.least];
    rest_points.extend(middle);
    rest_points.push(limits.top);
    println!("{:<22} {:>8} {:>8} {:>10} {:>12}", "", "mA", "mW", "frames/s", "mJ a frame");
    let mut at_rest = Vec::new();
    for khz in rest_points {
        let (ma, mw, _) = sample(&limits, khz, &mut |_| {});
        println!("{:<22} {ma:>8.0} {mw:>8.0}", format!("at rest, {} MHz", khz / 1000));
        at_rest.push((khz, mw));
    }
    for khz in [limits.least, limits.top] {
        let mut frames = 0u32;
        let (ma, mw, seconds) = sample(&limits, khz, &mut |until| {
            while Instant::now() < until {
                shell.tick(1.0 / 40.0);
                if frames % 12 == 0 {
                    shell.key(if frames % 24 == 0 { Key::Enter } else { Key::Back });
                }
                shell.draw();
                frames += 1;
            }
        });
        let fps = frames as f64 / seconds;
        let idle = at_rest.iter().find(|(k, _)| *k == khz).map_or(0.0, |(_, mw)| *mw);
        println!("{:<22} {ma:>8.0} {mw:>8.0} {fps:>10.1} {:>12.2}", format!("drawing, {} MHz", khz / 1000), (mw - idle) / fps.max(0.1));
    }
    if let (Some(low), Some(high)) = (at_rest.first(), at_rest.last()) {
        println!("the top at rest costs {:.0} mW more than the least (the turbo's core and memory clocks)", high.1 - low.1);
    }
    println!("(mJ a frame: what drawing adds to the draw at rest at that clock, over the frames it drew)");
    limits.restore();
}

/// The gauge's draw, in mA and mW, averaged once a second over 20 s at
/// `khz` (after 4 s to settle), while `work` runs until each second's end.
fn sample(limits: &Limits, khz: u32, work: &mut dyn FnMut(Instant)) -> (f64, f64, f64) {
    limits.pin(khz);
    let began = Instant::now();
    work(began + Duration::from_secs(4));
    let (mut ma, mut mw, mut n) = (0.0, 0.0, 0);
    let measured = Instant::now();
    for second in 1..=20u64 {
        let until = measured + Duration::from_secs(second);
        work(until);
        let now = Instant::now();
        if now < until {
            std::thread::sleep(until - now);
        }
        if let Some(r) = crate::power::Reading::read() {
            if let (Some(a), Some(v)) = (r.milliamps, r.volts) {
                ma += a as f64;
                mw += a as f64 * v as f64;
                n += 1;
            }
        }
    }
    let n = n.max(1) as f64;
    (ma / n, mw / n, measured.elapsed().as_secs_f64())
}

// -------------------------------------------------------------------- io ---

fn io() {
    let block = Path::new("/sys/block/mmcblk0/queue");
    println!("scheduler       {}", read(&block.join("scheduler")).unwrap_or_else(|| "– (no card here)".into()));
    for name in ["low_latency", "slice_idle", "back_seek_penalty", "fifo_expire_sync", "fifo_expire_async", "read_expire", "write_expire", "prio_aging_expire"] {
        if let Some(v) = read(&block.join("iosched").join(name)) {
            println!("  {name:<20} {v}");
        }
    }
    for name in ["dirty_background_bytes", "dirty_bytes", "dirty_expire_centisecs"] {
        println!("  vm.{name:<17} {}", read(&Path::new("/proc/sys/vm").join(name)).unwrap_or_else(|| "–".into()));
    }
    let dir = if Path::new("/data/catalyst").is_dir() { PathBuf::from("/data/catalyst") } else { std::env::temp_dir() };
    let reads = dir.join(".sched-bench-reads");
    // 32 MB to read 4 KB of at a time, from the card, not memory.
    let chunk = vec![0x5au8; 1 << 20];
    if std::fs::metadata(&reads).map_or(true, |m| m.len() < 32 << 20) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::File::create(&reads) {
            for _ in 0..32 {
                f.write_all(&chunk).ok();
            }
            f.sync_all().ok();
        }
    }
    for (phase, flood) in [("quiet", None), ("behind a song being saved", Some(Flood::Song)), ("behind an update being written", Some(Flood::Update))] {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writer = flood.map(|kind| {
            let (stop, file) = (stop.clone(), dir.join(".sched-bench-flood"));
            std::thread::spawn(move || kind.run(&file, &stop))
        });
        if writer.is_some() {
            std::thread::sleep(Duration::from_secs(3));
        }
        let (mut saves, mut uncached) = (Vec::new(), Vec::new());
        for i in 0..30u64 {
            let began = Instant::now();
            crate::sys::write_atomic(&dir.join(".sched-bench-save"), &chunk[..1024]).ok();
            saves.push(ms(began.elapsed()));
            if let Some(t) = read_uncached(&reads, (i * 7919 % 8000) * 4096) {
                uncached.push(t);
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let written = writer.and_then(|w| w.join().ok()).unwrap_or(0);
        println!("{phase}{}", if written > 0 { format!(" ({} MB written meanwhile)", written >> 20) } else { String::new() });
        println!("  a settings save  {}", spread(saves));
        println!("  an uncached read {}", spread(uncached));
    }
    for name in [".sched-bench-reads", ".sched-bench-save", ".sched-bench-flood"] {
        std::fs::remove_file(dir.join(name)).ok();
    }
}

/// 4 KB read from the card at `at`, the file's pages dropped from memory
/// first. Milliseconds; none off Linux (nothing to drop them with).
fn read_uncached(path: &Path, at: u64) -> Option<f64> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::FileExt;
        let f = std::fs::File::open(path).ok()?;
        // SAFETY: advice on a file this function holds open.
        unsafe { libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) };
        let mut buf = [0u8; 4096];
        let began = Instant::now();
        f.read_exact_at(&mut buf, at).ok()?;
        Some(ms(began.elapsed()))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (path, at);
        None
    }
}

#[derive(Clone, Copy)]
enum Flood {
    /// As Music saves a song: curl's class (nice 10, SCHED_BATCH) and the
    /// card's idle class, written as it comes, left to the kernel to flush.
    Song,
    /// As catalyst-update writes the spare system: SCHED_IDLE, the idle
    /// class, flushed every 4 MB.
    Update,
}

impl Flood {
    /// Write until told to stop (128 MB at most, then over again). Bytes written.
    fn run(self, file: &Path, stop: &std::sync::atomic::AtomicBool) -> u64 {
        use std::io::{Seek, Write};
        match self {
            Flood::Song => {
                crate::sched::enter(crate::sched::Class::Batch(10));
                #[cfg(target_os = "linux")]
                // SAFETY: this thread's own I/O class (idle).
                unsafe {
                    libc::syscall(libc::SYS_ioprio_set, 1, 0, 3 << 13);
                }
            }
            Flood::Update => crate::sched::enter(crate::sched::Class::Idle),
        }
        let Ok(mut f) = std::fs::File::create(file) else { return 0 };
        let chunk = vec![0xa5u8; 256 << 10];
        let (mut written, mut unflushed) = (0u64, 0usize);
        let began = Instant::now();
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            if f.write_all(&chunk).is_err() {
                break;
            }
            written += chunk.len() as u64;
            unflushed += chunk.len();
            // A song comes at Wi-Fi's pace: about 4 MB a second at best here.
            if matches!(self, Flood::Song) {
                let due = Duration::from_secs_f64(written as f64 / (4 << 20) as f64);
                if let Some(wait) = due.checked_sub(began.elapsed()) {
                    std::thread::sleep(wait);
                }
            }
            if matches!(self, Flood::Update) && unflushed >= 4 << 20 {
                f.sync_data().ok();
                unflushed = 0;
            }
            if f.stream_position().unwrap_or(0) >= 128 << 20 {
                f.rewind().ok();
            }
        }
        written
    }
}
