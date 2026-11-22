//! Music's manners: the programs it starts make way for the shell, and the
//! work it does in the background waits while memory, the battery or the
//! network are short.
//!
//! - **Helpers** (curl fetching, yt-dlp, QuickJS) run at nice 10 under
//!   SCHED_BATCH with an OOM score of 800, so the kernel ends them before
//!   anything that matters if memory runs out; the slow lookup is also
//!   capped in address space.
//! - **ffmpeg playing** runs at nice -10, ahead of drawing: the sound must
//!   not wait for a frame. (Not real-time: with a third of a second of ALSA
//!   buffer a nice level is enough, and a runaway real-time decoder could
//!   take the processor from the keyboard itself.) It too has OOM score 800:
//!   out of memory, the music stops before the shell does.
//! - **Background work** (looking ahead, downloads) waits while less than
//!   80 MB is free, the battery is low, or deep standby has begun.

use std::path::Path;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How a helper is run.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Manner {
    /// Fetching, looking up: nice 10, killed first.
    Background,
    /// The slow lookup (QuickJS answering YouTube's player): nice 15, killed
    /// first, and capped at this many MB of address space.
    Capped(u64),
    /// The decoder: nice -10, ahead of drawing; killed first all the same.
    Playing,
}

/// Prepare `cmd` to run in `manner`, and to end with the shell.
pub fn polite(cmd: &mut Command, manner: Manner) -> &mut Command {
    #[cfg(target_os = "linux")]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(move || {
            // Only what is safe between fork and exec: system calls, no allocation.
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            // Helpers are SCHED_BATCH as well (sched.rs): they never preempt
            // a frame on waking, and their timers fold into others'. The
            // decoder stays with the ordinary scheduler.
            match manner {
                Manner::Background => crate::sched::enter(crate::sched::Class::Batch(10)),
                Manner::Capped(_) => crate::sched::enter(crate::sched::Class::Batch(15)),
                Manner::Playing => {
                    libc::setpriority(libc::PRIO_PROCESS, 0, -10);
                }
            }
            // Downloads and lookups wait for the card behind everything else
            // (IOPRIO_WHO_PROCESS 1, this process, IOPRIO_CLASS_IDLE 3 << 13):
            // the screen never queues behind a song being saved.
            if manner != Manner::Playing {
                libc::syscall(libc::SYS_ioprio_set, 1, 0, 3 << 13);
            }
            let fd = libc::open(c"/proc/self/oom_score_adj".as_ptr(), libc::O_WRONLY);
            if fd >= 0 {
                libc::write(fd, b"800".as_ptr().cast(), 3);
                libc::close(fd);
            }
            if let Manner::Capped(mb) = manner {
                let limit = libc::rlimit { rlim_cur: mb * 1024 * 1024, rlim_max: mb * 1024 * 1024 };
                libc::setrlimit(libc::RLIMIT_AS, &limit);
            }
            Ok(())
        });
    }
    #[cfg(not(target_os = "linux"))]
    let _ = manner;
    cmd
}

/// This thread's own priority lowered (Linux keeps one for each thread):
/// for the shell's own threads that fetch and write in the background.
/// SCHED_BATCH at nice 10, with timer slack (sched.rs): what they fetch may
/// be the song playing, so they are never starved.
pub fn background_thread() {
    crate::sched::enter(crate::sched::Class::Batch(10));
}

/// Run to the end, or until `limit`: killed then, and said so.
pub fn output_within(cmd: &mut Command, limit: Duration) -> Result<Output, String> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("could not start: {e}"))?;
    // Read both pipes on threads, so a chatty program never blocks on a full one.
    let mut out = child.stdout.take();
    let mut err = child.stderr.take();
    let reading = std::thread::spawn(move || {
        use std::io::Read;
        let mut text = Vec::new();
        if let Some(o) = out.as_mut() {
            o.read_to_end(&mut text).ok();
        }
        text
    });
    let erroring = std::thread::spawn(move || {
        use std::io::Read;
        let mut text = Vec::new();
        if let Some(e) = err.as_mut() {
            e.read_to_end(&mut text).ok();
        }
        text
    });
    let started = Instant::now();
    let status: ExitStatus = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if started.elapsed() > limit {
            end(&mut child);
            reading.join().ok();
            erroring.join().ok();
            return Err("took too long".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let stdout = reading.join().unwrap_or_default();
    let stderr = erroring.join().unwrap_or_default();
    Ok(Output { status, stdout, stderr })
}

/// A child asked to stop (SIGTERM: ffmpeg closes the sound device cleanly),
/// then made to.
pub fn end(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    for _ in 0..20 {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    child.kill().ok();
    child.wait().ok();
}

// ------------------------------------------------------------- conditions ---

/// Background work waits with less than this free.
pub const MEMORY_FLOOR_MB: u64 = 80;
/// The slow lookup (QuickJS, about 200 MB) only with this much free.
pub const SLOW_LOOKUP_MB: u64 = 260;

/// Memory free to use, in MB (the kernel's MemAvailable).
pub fn memory_available_mb() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = text.lines().find(|l| l.starts_with("MemAvailable:"))?.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

pub fn memory_short() -> bool {
    memory_available_mb().is_some_and(|mb| mb < MEMORY_FLOOR_MB)
}

/// In deep standby (catalyst-power notes what it switched off there):
/// background work waits for it to wake. Music standby, where music plays
/// with the screen off, is not deep: songs looked ahead still come, so
/// Wi-Fi can rest once they have.
pub fn deep() -> bool {
    super::on_device() && Path::new("/run/catalyst-deep").exists() && !super::playing()
}

/// On the charger, and the charge: from the fuel gauge.
pub fn battery() -> Option<(bool, u32)> {
    let r = crate::power::reading_now()?;
    let charging = matches!(r.flow, crate::power::Flow::Charging | crate::power::Flow::Full | crate::power::Flow::Holding);
    Some((charging, r.percent))
}

/// The battery is low and not charging: background downloads wait.
pub fn battery_low() -> bool {
    battery().is_some_and(|(charging, percent)| !charging && percent <= 15)
}

/// Some way to the internet: a default route. Read from the kernel, no
/// program started. Off the handheld, always (the simulator's own catalogue).
pub fn online() -> bool {
    if !super::on_device() {
        return !OFFLINE_FOR_TESTS.load(Ordering::Relaxed);
    }
    std::fs::read_to_string("/proc/net/route").is_ok_and(|t| t.lines().skip(1).any(|l| l.split_whitespace().nth(1) == Some("00000000")))
}

/// Over Wi-Fi: the handheld's radio has an address (not the robot's cable,
/// not a computer's USB link).
pub fn on_wifi() -> bool {
    if !super::on_device() {
        return online();
    }
    Path::new("/sys/class/net/wlan0").exists() && crate::sys::address("wlan0").is_some_and(|a| a.contains('.'))
}

/// The simulator and tests pretend to be offline.
pub static OFFLINE_FOR_TESTS: AtomicBool = AtomicBool::new(false);
