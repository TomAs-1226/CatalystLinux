//! Offline dictation: speech written down on the handheld itself, with no
//! network and no key, by OpenAI's Whisper (tiny.en, English only) through
//! whisper.cpp's `whisper-server` (os/package/whisper-cpp). For the talk key
//! and whatever else dictates (voice.rs), which hands over speech already
//! trimmed to the words (the smart camera's voice-activity detector):
//!
//! - [`ready`]: whether speech can be written down now (the program, the
//!   model downloaded and checked, room in memory), or briefly why not.
//! - [`transcribe`]: what was said. Blocks (a second or two to load the
//!   model the first time, then about [`expected`]): call it from a thread.
//! - [`cancel`]: give up on the one under way (Esc); [`stop`]: unload now.
//! - [`away`] and [`give_way`]: for the shell, as the on-device model's.
//!
//! The model is downloaded when asked for, in Settings › Offline dictation,
//! to /data/models beside the on-device model's, pinned by size and
//! SHA-256: `ggml-tiny.en-q8_0.bin` from whisper.cpp's own conversions on
//! Hugging Face (44 MB, MIT). 8-bit rather than the 5-bit copy (32 MB):
//! measured on a PC over 74 LibriSpeech clips and JFK's, the 5-bit copy
//! took a quarter longer and got 10.9% of words wrong, the 8-bit 9.6%, the
//! 16-bit original 9.4% (and a sixth longer, in 33 MB more). 8-bit weights
//! need no unpacking, and the A53 has neither a dot-product instruction for
//! 5-bit nor fp16 arithmetic for 16-bit.
//!
//! **What it costs follows the speech.** Whisper encodes a 30 s window
//! (1500 positions, 50 a second) however short the clip; whisper-server
//! takes `audio_ctx` per request, so each asks for the speech's length plus
//! 2.6 s, and at least 6.4 s ([`audio_ctx`]): as right as the whole window
//! (9.6% against 9.4%) for a third of the work. Shorter, it starts saying
//! the same sentence again and again (half the words wrong on clips under
//! 3 s). What it writes is held to 8 tokens a second of speech
//! ([`max_tokens`], a patch of ours), and a sentence it still repeats is
//! kept once ([`squash_repeats`]). No temperature fallback: one decode,
//! whose cost is known.
//!
//! **How it runs**, by the on-device model's rules (local.rs):
//! - Nothing runs until speech needs writing down: never at boot, never
//!   because a page is open. The first transcription starts whisper-server
//!   (2 MB, from the image); a minute after the last ([`IDLE`]) it is
//!   stopped and its memory comes back.
//! - Nobody looking (the screen off, StandBy's faces, locked): it is
//!   stopped as soon as it is done.
//! - It never needs swap. It starts only with its 120 MB and the 60 MB
//!   below which waiting models give way free ([`room`]); waiting, it gives
//!   way then; and the on-device model, starting without room, asks it to
//!   give way first ([`give_way`]): it is back in a second or two, the
//!   language model in ten.
//! - Every thread of it runs at nice 10 under SCHED_BATCH from its first
//!   instruction (sched.rs's Batch: work someone waits on, which never
//!   preempts a frame), three of them, the fourth core the shell's; the
//!   kernel kills it first should memory run out. Waiting, it uses no
//!   processor time.
//! - It answers on 127.0.0.1:8089 only, one request at a time, and stops
//!   working on one the moment its asker goes ([`cancel`]).
//!
//! In the simulator there is no whisper-server: [`transcribe`] answers
//! [`DEMO_TEXT`] at about the handheld's pace, unless
//! `CATALYST_WHISPER_PORT` names one running on this machine (a PC build of
//! the same package).

use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bezel::{Align, Rect};
use localmodel::{self as lm, Pinned};

use crate::gfx::{Gfx, ListState, PAD, W};
use crate::local::{megabytes, Progress};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

// ------------------------------------------------------------------- model ---

/// The dictation model: one file, pinned.
pub struct Model {
    /// As people know it.
    pub name: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    /// Bytes, exactly.
    pub size: u64,
    /// whisper-server's resident memory at its largest (MB): 100 measured
    /// writing down sentences, 120 for a whole 30 s window.
    pub peak_mb: u32,
    pub licence: &'static str,
}

impl Pinned for Model {
    fn file(&self) -> &str {
        self.file
    }
    fn url(&self) -> &str {
        self.url
    }
    fn sha256(&self) -> &str {
        self.sha256
    }
    fn size(&self) -> u64 {
        self.size
    }
}

pub const MODEL: Model = Model {
    name: "Whisper tiny.en",
    file: "ggml-tiny.en-q8_0.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en-q8_0.bin",
    sha256: "5bc2b3860aa151a4c6e7bb095e1fcce7cf12c7b020ca08dcec0c6d018bb7dd94",
    size: 43_550_795,
    peak_mb: 120,
    licence: "MIT",
};

/// Where whisper-server listens: this handheld only, beside llama-server's
/// 8088.
pub const PORT: u16 = 8089;
/// The program, where the image has it.
pub const PROGRAM: &str = "/usr/bin/whisper-server";
const LOG: &str = "/tmp/whisper-server.log";
/// The rate Whisper hears at.
pub const RATE: u32 = speech::RATE;

/// What the simulator writes down, whatever it hears.
pub const DEMO_TEXT: &str = "Swap the intake belt before the next match.";
const NOTHING: &str = "nothing heard";

// ------------------------------------------------------------------ sizing ---

/// Encoder positions a second of speech: 100 mel frames, halved by the
/// encoder's second convolution.
pub const PER_SECOND: f32 = 50.0;
/// The whole window: 30 s.
pub const WINDOW: u32 = 1500;
/// The fewest positions asked for: 6.4 s. Fewer, and a short clip comes
/// back as its sentence over and over (measured: clips under 3 s, at 2 s
/// of window past them, had 82% of words wrong; at this, 12%, as with the
/// whole window).
pub const LEAST: u32 = 320;
/// Positions past the speech: 2.6 s of the silence whisper pads it with.
pub const MARGIN: u32 = 128;
/// What it may write a second of speech, in tokens: fast speech is about
/// four words, five or six tokens.
pub const TOKENS_PER_SECOND: f32 = 8.0;
/// Whisper's own limit: half its 448-token context.
pub const MOST_TOKENS: u32 = 224;
/// Threads: three of the four cores, as the on-device model; the fourth is
/// the shell's.
pub const THREADS: u32 = lm::THREADS;

/// The encoder positions to ask for, for `seconds` of speech: its length,
/// [`MARGIN`] past it and at least [`LEAST`]; 0 (whisper's whole window,
/// and 30 s windows one after another past that) when that would be all of
/// it anyway.
pub fn audio_ctx(seconds: f32) -> u32 {
    let speech = (seconds.max(0.0) * PER_SECOND).ceil() as u32;
    let ctx = (speech + MARGIN).max(LEAST);
    if ctx >= WINDOW {
        0
    } else {
        ctx
    }
}

/// The most tokens to write for `seconds` of speech (each 30 s window).
pub fn max_tokens(seconds: f32) -> u32 {
    ((TOKENS_PER_SECOND * seconds.clamp(0.0, 30.0)).ceil() as u32 + 8).min(MOST_TOKENS)
}

/// About how long the handheld takes to write down `seconds` of speech
/// with the model loaded (a second or two more to load it). An estimate
/// until measured there: whisper-server's time on a PC (Ryzen 7 9800X3D,
/// three threads: 44 ms for 2 s of speech, 59 for 5, 110 for 10) times 63,
/// how much slower the handheld's three cores read with the same ggml
/// (llama.cpp, SmolLM2 135M: 1247 tokens a second there, 19.8 here).
pub fn expected(seconds: f32) -> Duration {
    let ctx = match audio_ctx(seconds) {
        0 => WINDOW as f32,
        n => n as f32,
    };
    let tokens = 3.0 * seconds.clamp(0.0, 30.0) + 3.0;
    let pc_ms = 0.125 * ctx + 0.00002 * ctx * ctx + 0.7 * tokens;
    Duration::from_secs_f32(63.0 * pc_ms / 1000.0)
}

/// How whisper-server is run: the model, this handheld's address and the
/// port, [`THREADS`] threads, English. Everything else is asked per request
/// (`request_body`).
pub fn server_args(model_path: &Path) -> Vec<String> {
    ["-m", &model_path.to_string_lossy(), "--host", lm::HOST, "--port", &PORT.to_string(), "-t", &THREADS.to_string(), "-l", "en"].iter().map(|s| (*s).to_owned()).collect()
}

/// A request for `wav` (`seconds` of speech): multipart form data, as
/// whisper-server reads it, and the boundary it is cut by. Greedy, with no
/// temperature fallback, no timestamps, the window and the tokens sized to
/// the speech, sounds that are not speech ([MUSIC]) left out.
pub fn request_body(wav: &[u8], seconds: f32) -> (String, Vec<u8>) {
    let mut boundary = String::from("catalyst-dictation-5f3a9c");
    while wav.windows(boundary.len()).any(|w| w == boundary.as_bytes()) {
        boundary.push('x');
    }
    let fields = [
        ("response_format", "json".to_owned()),
        ("language", "en".to_owned()),
        ("temperature", "0".to_owned()),
        ("temperature_inc", "0".to_owned()),
        ("no_timestamps", "true".to_owned()),
        ("token_timestamps", "false".to_owned()),
        ("suppress_nst", "true".to_owned()),
        ("audio_ctx", audio_ctx(seconds).to_string()),
        ("max_tokens", max_tokens(seconds).to_string()),
    ];
    let mut body = Vec::with_capacity(wav.len() + 1024);
    for (name, value) in fields {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"speech.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes());
    body.extend_from_slice(wav);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (boundary, body)
}

/// What whisper-server answered, as the words said, or why there are none.
pub fn answer_text(status: u16, body: &str) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|_| format!("dictation answered {status} with something unreadable"))?;
    if let Some(why) = v["error"].as_str() {
        return Err(format!("dictation: {why}"));
    }
    if status != 200 {
        return Err(format!("dictation answered {status}"));
    }
    let text = squash_repeats(&tidy(v["text"].as_str().ok_or("dictation gave no text")?));
    if text.is_empty() {
        return Err(NOTHING.into());
    }
    Ok(text)
}

/// The words alone: one line, single spaces, no [BLANK_AUDIO] or other
/// bracketed marks, and none of the "(music)" whisper writes for a sound
/// with no words in it.
pub fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0;
    for c in text.chars() {
        match c {
            '[' => depth += 1,
            ']' if depth > 0 => depth -= 1,
            _ if depth > 0 => {}
            _ => out.push(c),
        }
    }
    let words = out.split_whitespace().collect::<Vec<_>>().join(" ");
    // Nothing but marks for sounds ("(music)", "*laughs*", "♪"): no words.
    let (mut bare, mut round, mut starred) = (String::new(), 0, false);
    for c in words.chars() {
        match c {
            '(' => round += 1,
            ')' if round > 0 => round -= 1,
            '*' => starred = !starred,
            _ if round == 0 && !starred => bare.push(c),
            _ => {}
        }
    }
    if bare.chars().any(char::is_alphanumeric) {
        words
    } else {
        String::new()
    }
}

/// A sentence (two words or more) said again and again at the end, as a
/// short window can set Whisper doing, kept once: "Where is that? Where is
/// that?" is "Where is that?". The last word may be cut short (the token
/// limit). Fewer than two whole repeats, or one word fewer than four times
/// ("no no", "bye bye"), are left as said.
pub fn squash_repeats(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let key = |w: &str| w.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let keys: Vec<String> = words.iter().map(|w| key(w)).collect();
    let n = keys.len();
    // The shortest text that the rest repeats: (where the repeating starts,
    // the length of what repeats).
    let mut best: Option<(usize, usize)> = None;
    for period in 1..=n / 2 {
        let same = |i: usize| keys[i] == keys[i - period] || (i == n - 1 && !keys[i].is_empty() && keys[i - period].starts_with(keys[i].as_str()));
        // Every word from `from` on repeats the one `period` before it.
        let mut from = n;
        while from > period && same(from - 1) {
            from -= 1;
        }
        let start = from - period;
        let repeated = n - start;
        let enough = if period == 1 { repeated >= 4 } else { repeated >= 2 * period };
        if enough && best.map_or(true, |(s, p)| start + period < s + p) {
            best = Some((start, period));
        }
    }
    match best {
        Some((start, period)) => words[..start + period].join(" "),
        None => words.join(" "),
    }
}

/// Whether there is room to start whisper-server with `available` MB free
/// (MemAvailable): its peak and the 60 MB below which a waiting model gives
/// its memory back, or what to close. (Unknown memory: let it try.)
pub fn room(available: Option<u64>) -> Result<(), String> {
    let need = u64::from(MODEL.peak_mb) + lm::SHORT_MB;
    match available {
        Some(free) if free < need => Err(format!("dictation needs {need} MB free and {free} MB is: close Music or other apps, or unload the on-device model")),
        _ => Ok(()),
    }
}

/// Whether dictation can be asked now, given where things stand: the
/// program in the image, the model downloaded, and room (or loaded already).
fn ready_with(program: bool, model: bool, loaded: bool, available: Option<u64>) -> Result<(), String> {
    if !program {
        return Err("whisper-server is not in this system: an update brings it".into());
    }
    if !model {
        return Err(format!("{} is not downloaded: Settings › Offline dictation", MODEL.name));
    }
    if loaded {
        return Ok(());
    }
    room(available)
}

// --------------------------------------------------------------- the server ---

/// Unasked for this long, whisper-server is stopped and its 100 MB given
/// back. A minute: dictation comes in bursts (a note, said a sentence at a
/// time), each after the first written at once, and loading again costs a
/// second or two.
pub const IDLE: Duration = Duration::from_secs(60);
/// A transcription just started counts as work this long.
const JUST_ASKED: Duration = Duration::from_secs(5);
/// Processor time in a second (clock ticks) that means it is working.
const WORKING_TICKS: u64 = 20;
/// Loading the model, from the card.
const START_WITHIN: Duration = Duration::from_secs(60);

struct Running {
    child: Child,
    last: Instant,
    cpu: u64,
}

/// Held while the model loads: never taken on the UI's thread.
static SERVER: Mutex<Option<Running>> = Mutex::new(None);
/// One transcription at a time.
static ASKING: Mutex<()> = Mutex::new(());
static REAPING: AtomicBool = AtomicBool::new(false);
/// The server's process id while one runs (0: none).
static PID: AtomicU32 = AtomicU32::new(0);
/// Transcriptions under way.
static WRITING: AtomicU32 = AtomicU32::new(0);
/// The one under way given up on.
static CANCEL: AtomicBool = AtomicBool::new(false);
/// Nobody is looking: stop once done.
static AWAY: AtomicBool = AtomicBool::new(false);
/// Asked to stop while loading: the load, once done, gives up.
static STOP_ASKED: AtomicBool = AtomicBool::new(false);

fn server() -> std::sync::MutexGuard<'static, Option<Running>> {
    SERVER.lock().unwrap_or_else(|e| e.into_inner())
}

/// A whisper-server already running on this machine to ask instead
/// (`CATALYST_WHISPER_PORT`: the simulator, a PC).
fn port_given() -> Option<u16> {
    std::env::var("CATALYST_WHISPER_PORT").ok().and_then(|p| p.trim().parse().ok())
}

/// The simulator and tests: no whisper-server, unless one is named.
fn demo() -> bool {
    crate::ai::demo() && port_given().is_none()
}

fn program() -> String {
    std::env::var("CATALYST_WHISPER_SERVER").unwrap_or_else(|_| PROGRAM.to_owned())
}

/// Whether dictation can write down speech now, or briefly why not. (The
/// talk key asks, voice.rs; until that lands, nothing here does.)
#[allow(dead_code)]
pub fn ready() -> Result<(), String> {
    if demo() || port_given().is_some() {
        return Ok(());
    }
    let dir = lm::dir();
    ready_with(Path::new(&program()).exists(), lm::present(&dir, &MODEL), PID.load(Ordering::Relaxed) != 0, lm::available_mb())
}

/// What was said in `samples` (mono, at `rate`), already trimmed to the
/// words. Blocks while the model loads (the first time) and writes; one at
/// a time. Speech of any rate is brought to Whisper's 16 kHz.
pub fn transcribe(samples: &[i16], rate: u32) -> Result<String, String> {
    if rate == 0 || samples.len() < (rate / 10) as usize {
        return Err(NOTHING.into());
    }
    let speech = to_whisper_rate(samples, rate);
    let seconds = speech.len() as f32 / RATE as f32;
    if demo() {
        if !cfg!(test) {
            std::thread::sleep(expected(seconds).min(Duration::from_secs(4)));
        }
        return Ok(DEMO_TEXT.into());
    }
    let _one = ASKING.lock().unwrap_or_else(|e| e.into_inner());
    CANCEL.store(false, Ordering::SeqCst);
    let _writing = Writing::new();
    let port = start()?;
    if CANCEL.load(Ordering::SeqCst) {
        return Err("stopped".into());
    }
    let wav = speech::wav_bytes(&speech, RATE);
    let (boundary, body) = request_body(&wav, seconds);
    let content_type = format!("multipart/form-data; boundary={boundary}");
    // Generous: the estimate is the handheld's pace with nothing else
    // running, and the frames come first.
    let timeout = Duration::from_secs(20) + 3 * expected(seconds);
    let request = lm::Request { port, method: "POST", path: "/inference", content_type: &content_type, body: &body, timeout, who: "dictation" };
    let answer = request.send(Some(&CANCEL));
    touch();
    match answer {
        Err(_) if CANCEL.load(Ordering::SeqCst) => Err("stopped".into()),
        Err(why) => Err(why),
        Ok((status, text)) => answer_text(status, &text),
    }
}

/// Speech at Whisper's rate.
fn to_whisper_rate(samples: &[i16], rate: u32) -> Vec<i16> {
    if rate == RATE {
        return samples.to_vec();
    }
    let mut out = Vec::with_capacity(samples.len() * RATE as usize / rate as usize + 64);
    let mut resampler = speech::Resampler::new(rate, 1, RATE);
    resampler.push(samples, &mut out);
    resampler.finish(&mut out);
    out
}

/// Counts a transcription as under way for as long as it lives.
struct Writing;

impl Writing {
    fn new() -> Self {
        WRITING.fetch_add(1, Ordering::SeqCst);
        AWAY.store(false, Ordering::SeqCst);
        Writing
    }
}

impl Drop for Writing {
    fn drop(&mut self) {
        WRITING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Give up on the transcription under way (Esc): its request is cut, and
/// whisper-server stops working on it.
pub fn cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

/// Bring whisper-server up if it is not (waiting while it loads); the port
/// it answers on.
fn start() -> Result<u16, String> {
    if let Some(port) = port_given() {
        return Ok(port);
    }
    let program = program();
    let dir = lm::dir();
    let mut running = server();
    STOP_ASKED.store(false, Ordering::SeqCst);
    if let Some(r) = running.as_mut() {
        if matches!(r.child.try_wait(), Ok(None)) && healthy(PORT) {
            r.last = Instant::now();
            return Ok(PORT);
        }
        if let Some(mut gone) = running.take() {
            PID.store(0, Ordering::SeqCst);
            lm::stop_server(&mut gone.child);
        }
    }
    ready_with(Path::new(&program).exists(), lm::present(&dir, &MODEL), false, lm::available_mb())?;
    // Something answers on the port already: a whisper-server a shell
    // before this one left running (it runs in a group of its own, and
    // outlives a shell that stops). Nothing would ever stop it: it goes.
    if healthy(PORT) {
        clear_strays();
    }
    let mut child = lm::spawn_background(&program, &server_args(&lm::path(&dir, &MODEL)), Path::new(LOG), Some(Path::new("/")))?;
    let started = Instant::now();
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("dictation stopped while starting ({status}); {LOG} says why"));
        }
        if healthy(PORT) {
            break;
        }
        if started.elapsed() > START_WITHIN || CANCEL.load(Ordering::SeqCst) {
            lm::stop_server(&mut child);
            return Err(if CANCEL.load(Ordering::SeqCst) { "stopped".into() } else { "dictation took too long to start".into() });
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if STOP_ASKED.swap(false, Ordering::SeqCst) {
        lm::stop_server(&mut child);
        return Err("stopped".into());
    }
    let pid = child.id();
    *running = Some(Running { child, last: Instant::now(), cpu: crate::local::cpu_time(pid) });
    PID.store(pid, Ordering::SeqCst);
    drop(running);
    reap_when_done();
    Ok(PORT)
}

/// Whether a program's command line (its arguments, NUL-separated, as
/// /proc has it) is a whisper-server on this module's port.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn is_ours(cmdline: &[u8]) -> bool {
    let args: Vec<&[u8]> = cmdline.split(|b| *b == 0).collect();
    let named = args.first().is_some_and(|a| a.rsplit(|b| *b == b'/').next() == Some(b"whisper-server".as_slice()));
    named && args.windows(2).any(|w| w[0] == b"--port" && w[1] == PORT.to_string().as_bytes())
}

/// Stop every whisper-server on this module's port that this shell did not
/// start (and so would never stop): asked, then made to.
fn clear_strays() {
    #[cfg(target_os = "linux")]
    {
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return;
        };
        let strays: Vec<i32> = entries
            .flatten()
            .filter_map(|e| e.file_name().to_str().and_then(|n| n.parse::<i32>().ok()))
            .filter(|pid| std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|c| is_ours(&c)))
            .collect();
        for &pid in &strays {
            // SAFETY: a signal to a process found by its command line.
            unsafe {
                libc::kill(pid, libc::SIGCONT);
                libc::kill(pid, libc::SIGTERM);
            }
        }
        for _ in 0..20 {
            if strays.iter().all(|pid| !Path::new(&format!("/proc/{pid}/cmdline")).exists() || std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|c| c.is_empty())) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        for &pid in &strays {
            // SAFETY: as above.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

fn healthy(port: u16) -> bool {
    let request = lm::Request { port, method: "GET", path: "/health", content_type: "application/json", body: b"", timeout: Duration::from_secs(2), who: "dictation" };
    request.send(None).is_ok_and(|(status, _)| status == 200)
}

/// Unload now, if this shell started it: its memory back. Never waits on a
/// load (it is called on the UI's thread): that is asked to give up once
/// done.
pub fn stop() {
    let mut running = match SERVER.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => {
            STOP_ASKED.store(true, Ordering::SeqCst);
            return;
        }
    };
    if let Some(mut r) = running.take() {
        PID.store(0, Ordering::SeqCst);
        lm::stop_server(&mut r.child);
    }
}

/// Settings' "unload now": the one under way given up on and the server
/// stopped, without the page waiting for it.
pub fn unload_now() {
    cancel();
    std::thread::Builder::new().name("dictation-stop".into()).stack_size(64 * 1024).spawn(stop).ok();
}

/// The on-device model is starting without room for it: dictation, loaded
/// and not writing, gives its memory back now. Whether it did.
pub fn give_way() -> bool {
    if PID.load(Ordering::SeqCst) == 0 || WRITING.load(Ordering::SeqCst) != 0 {
        return false;
    }
    let Ok(mut running) = SERVER.try_lock() else {
        return false;
    };
    match running.take() {
        Some(mut r) => {
            PID.store(0, Ordering::SeqCst);
            lm::stop_server(&mut r.child);
            true
        }
        None => false,
    }
}

/// Nobody is looking (the screen off, StandBy's faces, locked): stopped
/// within a second, or straight after the speech it is writing down. Cheap,
/// for the shell to call every frame while it is so.
pub fn away() {
    if PID.load(Ordering::Relaxed) != 0 {
        AWAY.store(true, Ordering::Relaxed);
    }
}

/// Just used: its idle time starts again.
fn touch() {
    if let Some(r) = server().as_mut() {
        r.last = Instant::now();
    }
}

/// The memory whisper-server holds now (MB), if this shell has it running.
pub fn loaded() -> Option<u64> {
    match PID.load(Ordering::Relaxed) {
        0 => None,
        pid => std::fs::read_to_string(format!("/proc/{pid}/status")).ok().and_then(|s| crate::local::resident_mb(&s)).or(Some(0)),
    }
}

/// Whether a server last working at `last` is to stop now: idle [`IDLE`],
/// or done while nobody is looking.
fn time_to_stop(last: Instant, now: Instant, away: bool) -> bool {
    let since = now.saturating_duration_since(last);
    since >= IDLE || (away && since >= JUST_ASKED)
}

/// A thread that looks at the server once a second and stops it when its
/// time has come, or when something else needs the memory while it waits.
fn reap_when_done() {
    if REAPING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("dictation".into())
        .stack_size(64 * 1024)
        .spawn(|| loop {
            std::thread::sleep(Duration::from_secs(1));
            let mut running = server();
            let now = Instant::now();
            let done = match running.as_mut() {
                None => true,
                Some(r) => {
                    let cpu = crate::local::cpu_time(r.child.id());
                    let working = cpu > r.cpu + WORKING_TICKS || WRITING.load(Ordering::SeqCst) != 0;
                    r.cpu = cpu;
                    if working {
                        r.last = now;
                    }
                    let short = !working && lm::memory_short(lm::available_mb());
                    !matches!(r.child.try_wait(), Ok(None)) || short || time_to_stop(r.last, now, AWAY.load(Ordering::SeqCst))
                }
            };
            if done {
                if let Some(mut r) = running.take() {
                    PID.store(0, Ordering::SeqCst);
                    lm::stop_server(&mut r.child);
                }
                AWAY.store(false, Ordering::SeqCst);
                REAPING.store(false, Ordering::SeqCst);
                break;
            }
        })
        .ok();
}

/// Settings' line for it.
pub fn summary() -> String {
    if loaded().is_some() {
        "loaded".into()
    } else if lm::present(&lm::dir(), &MODEL) {
        "ready".into()
    } else {
        "none yet".into()
    }
}

// --------------------------------------------------------------------- page ---

/// What the page has it say, to show it working: espeak says it, Whisper
/// writes it down.
const TRY: &str = DEMO_TEXT;

/// A try: what Whisper wrote, how long that took, how long the speech was,
/// and whether the model had to load first.
struct Tried {
    text: String,
    took: f32,
    speech: f32,
    loaded_first: bool,
}

impl Tried {
    fn words(&self) -> String {
        let how = if self.loaded_first { "loaded and written" } else { "written" };
        format!("{how} in {:.1} s for {:.1} s of speech", self.took, self.speech)
    }
}

/// `TRY`, said by espeak: (rate, samples).
fn spoken(text: &str) -> Result<(u32, Vec<i16>), String> {
    let procs = speech::Procs::default();
    let stop = AtomicBool::new(false);
    let (mut rate, mut all) = (0, Vec::new());
    speech::espeak::speak(text, &procs, &stop, &mut |r, s| {
        rate = r;
        all.extend_from_slice(s);
        Ok(())
    })?;
    procs.wait_all(&stop);
    if all.is_empty() || rate == 0 {
        return Err("espeak said nothing".into());
    }
    Ok((rate, all))
}

fn try_it() -> Result<Tried, String> {
    let (rate, samples) = if demo() { (RATE, vec![0i16; 3 * RATE as usize]) } else { spoken(TRY)? };
    let loaded_first = PID.load(Ordering::SeqCst) == 0 && port_given().is_none() && !demo();
    let started = Instant::now();
    let text = transcribe(&samples, rate)?;
    Ok(Tried { text, took: started.elapsed().as_secs_f32(), speech: samples.len() as f32 / rate as f32, loaded_first })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Not here: Enter downloads it.
    Away,
    Downloading,
    /// Here: Enter tries it.
    Here,
}

/// Settings › Offline dictation: the model, a row of its own, and what it
/// is above it. Enter downloads it, and once here has it write down a
/// sentence espeak says; d twice removes it (or stops its download); u
/// unloads it now. As Settings › On-device model, which it sits under.
pub struct DictationPage {
    dir: PathBuf,
    list: ListState,
    downloading: Option<(Arc<Progress>, Receiver<Result<(), String>>)>,
    trying: Option<Receiver<Result<Tried, String>>>,
    /// The last try's words and what it took, or what went wrong.
    answer: Option<(String, Option<String>)>,
    armed: bool,
    memory: Option<u64>,
    looked: f32,
    held: bool,
    clock: f32,
}

const LOOK_EVERY: f32 = 0.5;

impl DictationPage {
    pub fn new() -> Self {
        Self::at(lm::dir())
    }

    fn at(dir: PathBuf) -> Self {
        DictationPage { dir, list: ListState::default(), downloading: None, trying: None, answer: None, armed: false, memory: loaded(), looked: 0.0, held: false, clock: 0.0 }
    }

    fn state(&self) -> State {
        if self.downloading.is_some() {
            State::Downloading
        } else if lm::present(&self.dir, &MODEL) {
            State::Here
        } else {
            State::Away
        }
    }

    fn value(&self) -> String {
        match (self.state(), &self.downloading) {
            (State::Downloading, Some((p, _))) if p.checking.load(Ordering::Relaxed) => "checking".into(),
            (State::Downloading, Some((p, _))) => {
                let got = p.bytes.load(Ordering::Relaxed);
                format!("{}% · {} of {}", got * 100 / MODEL.size.max(1), megabytes(got), megabytes(MODEL.size))
            }
            (State::Here, _) => match self.memory {
                Some(mb) => format!("loaded · {mb} MB"),
                None => "downloaded".into(),
            },
            _ => megabytes(MODEL.size),
        }
    }

    fn start_download(&mut self) {
        let progress = Arc::new(Progress::default());
        progress.bytes.store(crate::local::len(&lm::partial(&self.dir, &MODEL)), Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        let (dir, shared) = (self.dir.clone(), progress.clone());
        std::thread::Builder::new()
            .name("dictation-download".into())
            .spawn(move || {
                // Nobody waits on it: the card's idle class (sched.rs).
                crate::sched::enter(crate::sched::Class::Idle);
                tx.send(crate::local::download(&MODEL, &dir, &shared)).ok();
            })
            .ok();
        self.downloading = Some((progress, rx));
        self.answer = None;
    }

    fn start_trying(&mut self) {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("dictation-try".into())
            .spawn(move || {
                // Someone waits on it: sched.rs's Batch, as the model itself.
                crate::sched::enter(crate::sched::Class::Batch(10));
                tx.send(try_it()).ok();
            })
            .ok();
        self.trying = Some(rx);
        self.answer = None;
    }

    fn remove(&mut self, cx: &mut Cx) {
        stop();
        lm::remove(&self.dir, &MODEL);
        self.answer = None;
        self.memory = None;
        *cx.say = Some(format!("{} removed · {} back", MODEL.name, megabytes(MODEL.size)));
    }

    fn unload(&mut self, cx: &mut Cx) {
        match self.memory.take() {
            Some(mb) => {
                unload_now();
                *cx.say = Some(format!("unloaded · {mb} MB back"));
            }
            None => *cx.say = Some("not loaded · it loads when you dictate".into()),
        }
    }

    fn working(&self) -> bool {
        self.downloading.is_some() || self.trying.is_some()
    }

    /// In the gallery: a download part way.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn showing_download(fraction: f64) -> Self {
        let mut page = Self::new();
        let progress = Arc::new(Progress::default());
        progress.bytes.store((MODEL.size as f64 * fraction) as u64, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        std::mem::forget(tx);
        page.downloading = Some((progress, rx));
        page
    }

    /// In the gallery: what a try wrote, and how long it took.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn showing_answer(text: &str, took: f32, speech: f32) -> Self {
        let mut page = Self::new();
        let tried = Tried { text: text.to_owned(), took, speech, loaded_first: false };
        page.answer = Some((tried.text.clone(), Some(tried.words())));
        page
    }

    /// In the gallery: loaded, holding `mb`.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn showing_loaded(mut self, mb: u64) -> Self {
        self.memory = Some(mb);
        self.held = true;
        self
    }
}

impl Screen for DictationPage {
    fn title(&self) -> &str {
        "Dictation"
    }

    fn busy(&self) -> bool {
        self.working() || self.memory.is_some()
    }

    fn keeps_awake(&self) -> bool {
        self.working()
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.clock += dt;
        self.looked += dt;
        let mut changed = false;
        if self.looked >= LOOK_EVERY && !self.held {
            self.looked = 0.0;
            let memory = loaded();
            changed = memory != self.memory;
            self.memory = memory;
        }
        self.working() || changed
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if let Some((_, rx)) = &self.downloading {
            match rx.try_recv() {
                Ok(result) => {
                    self.downloading = None;
                    *cx.say = Some(match result {
                        Ok(()) => format!("{} is ready · enter tries it", MODEL.name),
                        Err(why) => why,
                    });
                }
                Err(TryRecvError::Disconnected) => self.downloading = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = &self.trying {
            match rx.try_recv() {
                Ok(Ok(tried)) => {
                    *cx.say = Some(tried.words());
                    self.answer = Some((tried.text.clone(), Some(tried.words())));
                    self.trying = None;
                }
                Ok(Err(why)) => {
                    *cx.say = Some(why.clone());
                    self.answer = Some((why, None));
                    self.trying = None;
                }
                Err(TryRecvError::Disconnected) => self.trying = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        Nav::Stay
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let armed = std::mem::take(&mut self.armed);
        match key {
            Key::Back | Key::Leave => return Nav::Pop,
            Key::Enter | Key::Right => match self.state() {
                State::Away => self.start_download(),
                State::Downloading => *cx.say = Some("downloading · d twice stops it".into()),
                State::Here if self.trying.is_some() => *cx.say = Some("still writing".into()),
                State::Here => self.start_trying(),
            },
            Key::Char('d') => match self.state() {
                State::Downloading if armed => {
                    if let Some((progress, _)) = &self.downloading {
                        progress.cancel.store(true, Ordering::Relaxed);
                    }
                    *cx.say = Some("download stopped · it carries on from here next time".into());
                }
                State::Here if self.trying.is_some() => *cx.say = Some("still writing".into()),
                State::Here if armed => self.remove(cx),
                State::Downloading => {
                    self.armed = true;
                    *cx.say = Some("d again stops the download".into());
                }
                State::Here => {
                    self.armed = true;
                    *cx.say = Some(format!("d again removes {} from the card", MODEL.name));
                }
                State::Away => *cx.say = Some("not downloaded".into()),
            },
            Key::Char('u') => self.unload(cx),
            other => crate::screens::list_key(&mut self.list, other, 1),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let cap_lh = ty.caption.size + 4.0;
        let width = W - 2.0 * PAD;

        // What it is (two lines), or what a try wrote (up to three) and
        // what that took.
        let (mut lines, most, color) = match (&self.answer, &self.trying) {
            (_, Some(_)) => {
                let dots = ".".repeat(1 + (self.clock * 2.0) as usize % 3);
                (crate::help::wrap(g, &format!("espeak says a sentence, {} writes it down{dots}", MODEL.name), ty.caption, width), 2, t.dim)
            }
            (Some((text, Some(_))), None) => (crate::help::wrap(g, &format!("“{text}”"), ty.caption, width), 2, t.ink),
            (Some((why, None)), None) => (crate::help::wrap(g, why, ty.caption, width), 2, t.ink),
            _ => {
                let facts = [format!("{} · English · {}", MODEL.name, megabytes(MODEL.size)), format!("{} · {}", MODEL.name, megabytes(MODEL.size)), MODEL.name.to_owned()];
                let first = facts.iter().find(|f| g.measure(f, ty.caption) <= width).unwrap_or(&facts[2]).clone();
                (vec![first, format!("Speech to text, offline · {}", MODEL.licence)], 2, t.dim)
            }
        };
        if lines.len() > most {
            lines.truncate(most);
            if let Some(last) = lines.last_mut() {
                *last = g.text.fit(&format!("{last}…"), &ty.caption, width);
            }
        }
        // A try's timing, dim, on a line of its own under its words.
        let words = lines.len();
        if let (Some((_, Some(took))), None) = (&self.answer, &self.trying) {
            lines.push(g.text.fit(took, &ty.caption, width));
        }
        for (k, line) in lines.iter().enumerate() {
            let c = if k >= words { t.dim } else { color };
            g.text_at(PAD + 2.0, top + 1.0 + (k as f32 + 0.5) * cap_lh, Align::Left, line, ty.caption, c);
        }
        let rows_top = top + 4.0 + lines.len().max(2) as f32 * cap_lh;

        let state = self.state();
        for (i, y) in self.list.layout(g, 1, rows_top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let value_color = match state {
                State::Here => g.good(),
                State::Away => t.signal,
                State::Downloading => t.dim,
            };
            // "Model": its name is above, and a download's progress needs
            // the row's width at the largest text size.
            g.row(r, "Model", &self.value(), value_color, i == self.list.focus);
            if let Some((p, _)) = &self.downloading {
                let fraction = p.bytes.load(Ordering::Relaxed) as f32 / MODEL.size.max(1) as f32;
                g.meter(Rect::new(r.x + 12.0, r.bottom() - 3.5, r.w - 24.0, 2.0), fraction, t.signal);
            }
        }
        self.list.scrollbar(g, 1, rows_top, bottom);
        match state {
            State::Downloading => g.hint_any(&["d twice: stop · it carries on later", "d twice: stop"]),
            State::Away => g.hint_any(&["enter: download from Hugging Face", "enter: download"]),
            State::Here if self.memory.is_some() => g.hint_any(&["enter: try it · u: unload now · d d: remove", "enter: try · u: unload now", "u: unload now"]),
            State::Here => g.hint_any(&["enter: try it · d twice: remove", "enter: try it · d d: remove"]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_is_pinned_and_small() {
        assert_eq!(MODEL.sha256.len(), 64);
        assert!(MODEL.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert!(MODEL.url.starts_with("https://huggingface.co/ggerganov/whisper.cpp/resolve/") && MODEL.url.ends_with(MODEL.file));
        assert_eq!((MODEL.file, MODEL.size), ("ggml-tiny.en-q8_0.bin", 43_550_795), "the 8-bit tiny.en, as measured");
        assert_eq!(MODEL.licence, "MIT");
        // Beside the shell and a loaded language model, in RAM alone.
        let balanced = &lm::MODELS[lm::DEFAULT];
        assert!(MODEL.peak_mb + lm::SHELL_MB + balanced.peak_mb + lm::SHORT_MB as u32 <= lm::RAM_MB);
        assert_ne!(PORT, lm::PORT, "not llama-server's port");
        assert!(!lm::RETIRED.contains(&MODEL.file) && lm::MODELS.iter().all(|m| m.file != MODEL.file));
    }

    #[test]
    fn the_window_follows_the_speech() {
        // Short speech: at least 6.4 s of window, never the whole 30 s.
        assert_eq!(audio_ctx(0.0), LEAST);
        assert_eq!(audio_ctx(1.6), LEAST);
        assert_eq!(audio_ctx(3.84), LEAST, "192 + 128 = 320");
        // Then the speech and 2.6 s past it, rounded up to whole positions.
        assert_eq!(audio_ctx(4.0), 200 + MARGIN);
        assert_eq!(audio_ctx(4.01), 201 + MARGIN);
        assert_eq!(audio_ctx(10.0), 500 + MARGIN);
        // From 27.4 s, all of it: 0 asks for whisper's whole window, and
        // windows one after another past 30 s.
        assert_eq!(audio_ctx(27.4), 1498);
        assert_eq!(audio_ctx(27.44), 0);
        assert_eq!(audio_ctx(45.0), 0);
        assert_eq!(audio_ctx(-1.0), LEAST, "nonsense is short");
        assert!((0..300).map(|i| i as f32 / 10.0).all(|s| audio_ctx(s) == 0 || audio_ctx(s) < WINDOW));
        // A sentence costs a fraction of the window.
        assert!(audio_ctx(4.0) * 4 < WINDOW);
    }

    #[test]
    fn what_it_writes_is_held_to_the_length_of_the_speech() {
        assert_eq!(max_tokens(0.0), 8);
        assert_eq!(max_tokens(4.0), 40);
        assert_eq!(max_tokens(2.1), 25);
        assert_eq!(max_tokens(30.0), MOST_TOKENS, "a whole window: whisper's own limit");
        assert_eq!(max_tokens(300.0), MOST_TOKENS, "each window, however long the speech");
        // Fast speech (four words a second, about six tokens) fits.
        assert!((1..30).all(|s| max_tokens(s as f32) as f32 >= 6.0 * s as f32));
    }

    #[test]
    fn the_handheld_is_expected_to_take_about_as_long_as_was_said() {
        let four = expected(4.0).as_secs_f32();
        assert!((2.5..4.5).contains(&four), "{four}");
        // Longer speech, longer; but less than in proportion past the least window.
        assert!(expected(10.0) > expected(4.0) && expected(4.0) >= expected(1.0));
        assert!(expected(10.0).as_secs_f32() < 10.0);
        // The whole window, as whisper.cpp's own benchmark has it on slow ARM: many seconds.
        assert!(expected(29.0).as_secs_f32() > 12.0);
    }

    #[test]
    fn only_a_whisper_server_on_this_port_is_taken_for_a_stray() {
        let line = |args: &[&str]| args.join("\0").into_bytes();
        assert!(is_ours(&line(&["/usr/bin/whisper-server", "-m", "/data/models/x.bin", "--host", "127.0.0.1", "--port", "8089", "-t", "3"])));
        assert!(is_ours(&line(&["whisper-server", "--port", "8089"])));
        assert!(!is_ours(&line(&["/usr/bin/whisper-server", "--port", "8090"])), "another port: someone else's");
        assert!(!is_ours(&line(&["/usr/bin/llama-server", "--port", "8089"])));
        assert!(!is_ours(&line(&["/bin/sh", "-c", "whisper-server --port 8089"])), "a shell that runs one is not one");
        assert!(!is_ours(b""), "a process gone");
    }

    #[test]
    fn the_server_listens_on_this_handheld_only_with_three_threads() {
        let args = server_args(Path::new("/data/models/ggml-tiny.en-q8_0.bin"));
        let at = |flag: &str| args.iter().position(|a| a == flag).map(|i| args[i + 1].as_str());
        assert_eq!(at("-m"), Some("/data/models/ggml-tiny.en-q8_0.bin"));
        assert_eq!(at("--host"), Some("127.0.0.1"));
        assert_eq!(at("--port"), Some("8089"));
        assert_eq!(at("-t"), Some("3"), "the fourth core is the shell's");
        assert_eq!(at("-l"), Some("en"));
    }

    /// The fields of a multipart body, by name.
    fn fields(body: &[u8], boundary: &str) -> Vec<(String, Vec<u8>)> {
        let marker = format!("--{boundary}");
        let mut out = Vec::new();
        let text = body;
        let mut at = 0;
        let find = |hay: &[u8], needle: &[u8], from: usize| hay[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from);
        while let Some(start) = find(text, marker.as_bytes(), at) {
            let head_start = start + marker.len();
            if text[head_start..].starts_with(b"--") {
                break;
            }
            let head_end = find(text, b"\r\n\r\n", head_start).unwrap();
            let head = String::from_utf8_lossy(&text[head_start..head_end]).into_owned();
            let name = head.split("name=\"").nth(1).unwrap().split('"').next().unwrap().to_owned();
            let next = find(text, marker.as_bytes(), head_end).unwrap();
            out.push((name, text[head_end + 4..next - 2].to_vec()));
            at = next;
        }
        out
    }

    #[test]
    fn a_request_is_sized_to_the_speech_and_carries_it_whole() {
        let samples: Vec<i16> = (0..RATE as i32 * 4).map(|i| ((i * 37) % 2000 - 1000) as i16).collect();
        let wav = speech::wav_bytes(&samples, RATE);
        let (boundary, body) = request_body(&wav, 4.0);
        let parts = fields(&body, &boundary);
        let get = |name: &str| parts.iter().find(|(n, _)| n == name).map(|(_, v)| String::from_utf8_lossy(v).into_owned());
        assert_eq!(get("audio_ctx").as_deref(), Some("328"));
        assert_eq!(get("max_tokens").as_deref(), Some("40"));
        assert_eq!(get("temperature_inc").as_deref(), Some("0"), "one decode, no fallbacks");
        assert_eq!(get("no_timestamps").as_deref(), Some("true"));
        assert_eq!(get("response_format").as_deref(), Some("json"));
        assert_eq!(get("language").as_deref(), Some("en"));
        assert_eq!(parts.iter().find(|(n, _)| n == "file").unwrap().1, wav, "the sound, byte for byte");
        assert!(body.ends_with(format!("--{boundary}--\r\n").as_bytes()));
        // A boundary that happens to be in the sound is not used.
        let mut tricky = wav.clone();
        tricky.extend_from_slice(b"catalyst-dictation-5f3a9c");
        let (other, body) = request_body(&tricky, 4.0);
        assert_ne!(other, boundary);
        assert_eq!(fields(&body, &other).last().unwrap().1, tricky);
    }

    #[test]
    fn the_answer_is_the_words_alone() {
        assert_eq!(answer_text(200, r#"{"text":" And so my fellow Americans,\n ask not.\n"}"#).unwrap(), "And so my fellow Americans, ask not.");
        assert_eq!(answer_text(200, r#"{"text":"[BLANK_AUDIO]"}"#).unwrap_err(), NOTHING);
        assert_eq!(answer_text(200, r#"{"text":" (upbeat music)"}"#).unwrap_err(), NOTHING);
        assert_eq!(answer_text(200, r#"{"text":"Where is that? Where is that?"}"#).unwrap(), "Where is that?");
        assert!(answer_text(400, r#"{"error":"no 'file' field in the request"}"#).unwrap_err().contains("no 'file' field"));
        assert!(answer_text(500, "<html>").unwrap_err().contains("500"));
        assert!(answer_text(503, r#"{"status":"loading model"}"#).unwrap_err().contains("503"));
    }

    #[test]
    fn marks_for_sounds_go_and_words_stay() {
        assert_eq!(tidy(" Swap the belt [BLANK_AUDIO] now. "), "Swap the belt now.");
        assert_eq!(tidy("*laughs*"), "");
        assert_eq!(tidy("♪ ♪"), "");
        assert_eq!(tidy("(music) (applause)"), "");
        // Brackets in what was said are words.
        assert_eq!(tidy("Check the CAN bus (left side) first."), "Check the CAN bus (left side) first.");
        assert_eq!(tidy("Q32, 4.5 volts"), "Q32, 4.5 volts");
    }

    #[test]
    fn a_sentence_said_over_and_over_is_kept_once() {
        assert_eq!(squash_repeats("Where is that? Where is that?"), "Where is that?");
        assert_eq!(squash_repeats("the 20s the 20s the 20s the 20s the 20s the 20"), "the 20s");
        assert_eq!(squash_repeats("He doesn't work at all. He doesn't work at all. He doesn't w"), "He doesn't work at all.");
        assert_eq!(squash_repeats("Oh no, I'm quite sure. Oh no, I'm quite sure."), "Oh no, I'm quite sure.");
        // A repeat that starts after something else keeps what came first.
        assert_eq!(squash_repeats("Look at it. The belt. The belt. The belt."), "Look at it. The belt.");
        // What people say twice, or a word a few times, stays.
        assert_eq!(squash_repeats("no no"), "no no");
        assert_eq!(squash_repeats("no no no"), "no no no");
        assert_eq!(squash_repeats("bye bye"), "bye bye");
        assert_eq!(squash_repeats("very very very very"), "very");
        assert_eq!(squash_repeats("And so my fellow Americans ask not what your country can do for you, ask what you can do for your country."), "And so my fellow Americans ask not what your country can do for you, ask what you can do for your country.");
        assert_eq!(squash_repeats("the cat sat on the cat sat"), "the cat sat on the cat sat");
        assert_eq!(squash_repeats(""), "");
    }

    #[test]
    fn it_starts_only_with_room_and_says_what_is_missing() {
        let need = u64::from(MODEL.peak_mb) + lm::SHORT_MB;
        assert!(room(Some(need)).is_ok());
        let why = room(Some(need - 1)).unwrap_err();
        assert!(why.contains(&format!("{need} MB")) && why.contains("Music") && why.contains("on-device model"), "{why}");
        assert!(room(None).is_ok(), "unknown memory: it may try");
        // At rest the handheld has about 370 MB free; with the Balanced
        // language model loaded (186 MB held), about 185: room either way.
        assert!(room(Some(370)).is_ok() && room(Some(185)).is_ok());
        assert!(ready_with(false, true, false, None).unwrap_err().contains("whisper-server"));
        assert!(ready_with(true, false, false, None).unwrap_err().contains("Settings › Offline dictation"));
        assert!(ready_with(true, true, true, Some(10)).is_ok(), "loaded already: nothing more to find room for");
        assert!(ready_with(true, true, false, Some(10)).is_err());
        assert!(ready_with(true, true, false, Some(370)).is_ok());
    }

    #[test]
    fn an_unasked_server_stops_after_a_minute_and_at_once_when_nobody_looks() {
        let start = Instant::now();
        let after = |s: u64| start + Duration::from_secs(s);
        assert!(!time_to_stop(start, after(59), false));
        assert!(time_to_stop(start, after(60), false));
        assert!(!time_to_stop(after(10), start, false), "a clock that went back is not idle");
        assert!(!time_to_stop(start, after(2), true), "not between being started and asked");
        assert!(time_to_stop(start, start + JUST_ASKED, true));
        assert!(IDLE < Duration::from_secs(120), "less than the language model's two minutes: it is back sooner");
    }

    #[test]
    fn the_simulator_writes_down_its_sentence() {
        crate::shell::tests::sandbox();
        assert!(ready().is_ok());
        let speech = vec![0i16; 2 * RATE as usize];
        assert_eq!(transcribe(&speech, RATE).unwrap(), DEMO_TEXT);
        // Any rate, brought to Whisper's.
        assert_eq!(transcribe(&vec![0i16; 44_100], 44_100).unwrap(), DEMO_TEXT);
        assert!(to_whisper_rate(&vec![0i16; 48_000], 48_000).len().abs_diff(RATE as usize) < 200, "a second is a second");
        // Too short to be anything.
        assert_eq!(transcribe(&[0i16; 100], RATE).unwrap_err(), NOTHING);
        assert_eq!(transcribe(&speech, 0).unwrap_err(), NOTHING);
        // Nothing was started for it, and nothing gives way.
        assert!(loaded().is_none() && !give_way());
        stop();
        away();
        cancel();
    }

    /// With a real whisper-server: on a PC, a build of os/package/whisper-cpp
    /// (its patches applied) and the model. Either one already running
    /// (`CATALYST_WHISPER_PORT`), or, on Linux, the program
    /// (`CATALYST_WHISPER_SERVER`) and a models folder holding the model
    /// and its `.checked` mark (`CATALYST_MODELS`), started and stopped as
    /// on the handheld (with `CATALYST_AI_LIVE=1`, so that the simulator's
    /// stand-in steps aside). `CATALYST_WHISPER_WAV` names speech to write
    /// down and `CATALYST_WHISPER_SAYS` what it says. `cargo test --
    /// --ignored writes_down_real_speech`.
    #[test]
    #[ignore]
    fn writes_down_real_speech() {
        let (Ok(wav), Ok(says)) = (std::env::var("CATALYST_WHISPER_WAV"), std::env::var("CATALYST_WHISPER_SAYS")) else {
            eprintln!("no CATALYST_WHISPER_WAV and CATALYST_WHISPER_SAYS: skipped");
            return;
        };
        let samples = speech::read_wav(&std::fs::read(wav).unwrap()).unwrap();
        let words = |t: &str| t.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_owned).collect::<Vec<_>>();
        let spawned = port_given().is_none();
        // One left running by a shell before (on the port, not this one's):
        // it goes, and this shell's own answers.
        let mut stray = None;
        if spawned {
            assert!(ready_with(Path::new(&program()).exists(), lm::present(&lm::dir(), &MODEL), false, None).is_ok(), "{:?}", ready());
            let mut left = std::process::Command::new(program()).args(server_args(&lm::path(&lm::dir(), &MODEL))).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap();
            let up = Instant::now();
            while !healthy(PORT) {
                assert!(up.elapsed() < Duration::from_secs(30) && left.try_wait().unwrap().is_none(), "the stray never came up");
                std::thread::sleep(Duration::from_millis(50));
            }
            stray = Some(left);
        }
        for round in 0..2 {
            let started = Instant::now();
            let text = transcribe(&samples, RATE).unwrap();
            eprintln!("round {round}: {:.2} s for {:.1} s of speech: {text}", started.elapsed().as_secs_f32(), samples.len() as f32 / RATE as f32);
            assert_eq!(words(&text), words(&says));
            if spawned {
                assert!(loaded().is_some_and(|mb| mb > 40), "loaded and holding the model: {:?}", loaded());
            }
            if let Some(mut left) = stray.take() {
                assert!(left.try_wait().unwrap().is_some(), "the stray was stopped");
            }
        }
        // At 8 kHz too, brought to Whisper's rate.
        let half: Vec<i16> = samples.iter().step_by(2).copied().collect();
        assert_eq!(words(&transcribe(&half, RATE / 2).unwrap()), words(&says));
        if spawned {
            // Run as on the handheld: nice 10 under SCHED_BATCH (3), first
            // for the out-of-memory killer, in / (where ggml finds no
            // libggml-*.so of llama's to load).
            #[cfg(target_os = "linux")]
            {
                let pid = PID.load(Ordering::SeqCst);
                let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
                let fields: Vec<&str> = stat.rsplit_once(')').unwrap().1.split_whitespace().collect();
                assert_eq!((fields[16], fields[38]), ("10", "3"), "nice and policy");
                assert_eq!(std::fs::read_to_string(format!("/proc/{pid}/oom_score_adj")).unwrap().trim(), "800");
                assert_eq!(std::fs::read_link(format!("/proc/{pid}/cwd")).unwrap(), Path::new("/"));
                // Every thread too, once one is working.
                for task in std::fs::read_dir(format!("/proc/{pid}/task")).unwrap() {
                    let stat = std::fs::read_to_string(task.unwrap().path().join("stat")).unwrap();
                    let fields: Vec<&str> = stat.rsplit_once(')').unwrap().1.split_whitespace().collect();
                    assert_eq!((fields[16], fields[38]), ("10", "3"));
                }
            }
            // Given up on while it writes (twice the speech, so it is still
            // writing): the asker goes, the server stops, and the next is
            // written as ever.
            let long: Vec<i16> = samples.iter().chain(samples.iter()).copied().collect();
            let giving_up = std::thread::spawn(|| {
                std::thread::sleep(Duration::from_millis(30));
                cancel();
            });
            let started = Instant::now();
            assert_eq!(transcribe(&long, RATE).unwrap_err(), "stopped");
            eprintln!("given up on after {:.2} s", started.elapsed().as_secs_f32());
            giving_up.join().unwrap();
            assert_eq!(words(&transcribe(&samples, RATE).unwrap()), words(&says));
            stop();
            assert!(loaded().is_none(), "stopped: its memory back");
        }
    }

    #[test]
    fn the_page_downloads_tries_and_removes_the_model() {
        crate::shell::tests::sandbox();
        let dir = std::env::temp_dir().join(format!("catalyst-dictation-page-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let mut page = DictationPage::at(dir.clone());
        page.memory = None;
        page.held = true;
        assert!(SERVER.try_lock().map_or(true, |s| s.is_none()), "the page started nothing");
        assert_eq!(page.state(), State::Away);
        assert_eq!(page.value(), megabytes(MODEL.size));
        // Downloaded (a stand-in of its size, accepted as checked).
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::File::create(lm::partial(&dir, &MODEL)).unwrap().set_len(MODEL.size).unwrap();
        lm::accept(&dir, &MODEL, MODEL.sha256).unwrap();
        assert_eq!((page.state(), page.value()), (State::Here, "downloaded".to_owned()));
        page.memory = Some(98);
        assert_eq!(page.value(), "loaded · 98 MB");

        let (mut robot, mut settings, mut say) = (Robot::demo(), crate::shell::Settings::default(), None);
        let mut cx = Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true };
        page.key(Key::Char('u'), &mut cx);
        assert_eq!(cx.say.take().unwrap(), "unloaded · 98 MB back");
        page.key(Key::Char('u'), &mut cx);
        assert!(cx.say.take().unwrap().contains("not loaded"));
        // Enter tries it: the simulator's sentence comes back, timed.
        page.key(Key::Enter, &mut cx);
        let started = Instant::now();
        while page.trying.is_some() {
            page.update(&mut cx);
            assert!(started.elapsed() < Duration::from_secs(10), "the try never came back");
            std::thread::sleep(Duration::from_millis(5));
        }
        let (words, took) = page.answer.clone().unwrap();
        assert_eq!(words, DEMO_TEXT);
        assert!(took.unwrap().contains("s of speech"));
        // d once warns; anything between disarms; d twice removes.
        page.key(Key::Char('d'), &mut cx);
        assert!(cx.say.take().unwrap().contains("d again removes"));
        page.key(Key::Down, &mut cx);
        page.key(Key::Char('d'), &mut cx);
        assert!(lm::present(&dir, &MODEL), "not straight after: kept");
        page.key(Key::Char('d'), &mut cx);
        assert!(!lm::present(&dir, &MODEL), "d twice removes it");
        assert!(cx.say.take().unwrap().contains("removed"));
        assert_eq!(page.state(), State::Away);
        std::fs::remove_dir_all(&dir).ok();
    }
}
