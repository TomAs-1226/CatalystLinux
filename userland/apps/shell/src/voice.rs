//! Voice: push-to-talk, dictation and spoken answers.
//!
//! **The talk key is Alt** (bottom row, between Ctrl and Z). It is the one
//! key on the CardputerZero the shell had no use for in any layer: it types
//! nothing, so holding it never puts a letter into a text field or a
//! terminal; it is not one of the keyboard driver's Shift, Fn or Sym keys
//! (whose tap, double-tap and hold the driver itself interprets), so its
//! press and release reach the shell as they happen; and it sits under the
//! left thumb. A USB keyboard's either Alt does the same.
//!
//! - **Held anywhere**, the island listens: a microphone, what letting go
//!   does, and a live level meter. Let go, and what was said is written
//!   down (OpenAI's `gpt-transcribe`, with the OpenAI key from Settings ›
//!   Keys) and goes to the screen in front if it takes dictation, otherwise
//!   to the assistant, which grows out of the island with it as its question.
//! - **In the assistant** the words land in its line, shown and editable for
//!   a moment before they send themselves (any key stops that and edits).
//! - **In Notes** (the editor a note opens in) they are typed at the cursor;
//!   on Notes' list they start a new note.
//! - **Spoken answers** (off unless chosen, in the assistant's choices):
//!   OpenAI's `gpt-audio-1.5` with the voice chosen, or espeak on the
//!   handheld. A short answer is read as it is; a long one is said in a
//!   couple of sentences. Any key stops it; Esc does nothing else.
//!
//! The microphone is only open while the key is held (and a moment after,
//! for the last word), and the speaker only while an answer is said: each is
//! a program of ALSA's own (`arecord`, `aplay`), stopped the moment it is
//! done with, so the codec's input and the amplifier power down in between
//! and nothing of this runs at rest. What was heard goes to OpenAI through
//! curl's stdin and is never written anywhere; the question it becomes is
//! kept with the conversation, as a typed one is.
//!
//! Writing speech down needs the OpenAI key and the internet. Not offline:
//! whisper.cpp's smallest model (tiny, about 75 MB in use) encodes a whole
//! 30 s window however short the clip, which its own benchmark table puts at
//! 13.8 s on a Raspberry Pi 4 (an older build; the Pi 4 is about three times
//! this A53), so several seconds at best here for each thing said: too slow
//! to talk to. The offline voice is espeak, not a neural one (Piper needs
//! ONNX Runtime, which Buildroot does not package).
//!
//! **For other apps**: a screen takes dictation by answering
//! [`crate::shell::Screen::dictation`] with [`Dictation::Text`] and taking
//! the words in `dictated`; the shell does the rest (the key, the meter, the
//! transcription). For a flow of its own, [`listen`] opens the microphone
//! and [`dictate`] turns what it heard into text.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bezel::Rect;
use detent::{role, Animated};
use speech::sink::Sink;
use speech::source::Source;
use speech::{Clip, Procs};

use crate::gfx::Gfx;
use crate::keys::Keys;
use crate::platform::Key;

// ----------------------------------------------------------------- the key ---

/// The talk key's codes: KEY_LEFTALT, and a USB keyboard's KEY_RIGHTALT.
pub const TALK_CODES: [u16; 2] = [56, 100];

thread_local! {
    /// Whether the talk key is down, as the platform last saw it. On the
    /// shell's own thread, where keys are read and the shell runs (so each
    /// test, on its own thread, has its own key).
    static TALK_DOWN: Cell<bool> = const { Cell::new(false) };
}

/// The platform saw the talk key go down or come up.
pub fn set_talk_key(down: bool) {
    TALK_DOWN.with(|k| k.set(down));
}

pub fn talk_key_down() -> bool {
    TALK_DOWN.with(Cell::get)
}

/// Whether a key code is the talk key.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn is_talk_code(code: u16) -> bool {
    TALK_CODES.contains(&code)
}

/// What a screen does with what was said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dictation {
    /// A question, for the assistant (the assistant itself takes these).
    Question,
    /// Words typed in where the cursor is.
    Text,
}

// ------------------------------------------------------------------ choices ---

/// Whether answers are said aloud.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Speak {
    #[default]
    Off,
    /// Only the answer to a question asked aloud.
    Asked,
    Always,
}

impl Speak {
    fn key(self) -> &'static str {
        match self {
            Speak::Off => "off",
            Speak::Asked => "asked",
            Speak::Always => "always",
        }
    }

    pub fn words(self) -> &'static str {
        match self {
            Speak::Off => "off",
            Speak::Asked => "when asked aloud",
            Speak::Always => "always",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Speak::Off => Speak::Asked,
            Speak::Asked => Speak::Always,
            Speak::Always => Speak::Off,
        }
    }
}

/// Who says them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Voice {
    /// One of OpenAI's voices ([`speech::openai::VOICES`]).
    OpenAi(&'static str),
    /// espeak, on the handheld.
    Device,
}

impl Default for Voice {
    fn default() -> Self {
        Voice::OpenAi(speech::openai::VOICES[0])
    }
}

impl Voice {
    fn key(self) -> &'static str {
        match self {
            Voice::OpenAi(name) => name,
            Voice::Device => "espeak",
        }
    }

    /// In the choices' row: "marin · openai".
    pub fn words(self) -> String {
        match self {
            Voice::OpenAi(name) => format!("{name} · openai"),
            Voice::Device => "espeak · offline".into(),
        }
    }

    /// What it is, for the hint under the row (OpenAI's voices are AI voices,
    /// and are said to be).
    pub fn about(self) -> &'static str {
        match self {
            Voice::OpenAi(_) => "an ai voice from openai · uses the openai key",
            Voice::Device if speech::espeak::available() || crate::ai::demo() => "espeak on the handheld · offline, robotic",
            Voice::Device => "espeak is not on this handheld",
        }
    }

    pub fn next(self) -> Self {
        let voices = speech::openai::VOICES;
        match self {
            Voice::OpenAi(name) => match voices.iter().position(|v| *v == name) {
                Some(i) if i + 1 < voices.len() => Voice::OpenAi(voices[i + 1]),
                _ => Voice::Device,
            },
            Voice::Device => Voice::OpenAi(voices[0]),
        }
    }
}

/// The voice choices, in `config_dir()/voice`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Prefs {
    pub speak: Speak,
    pub voice: Voice,
}

fn prefs_path() -> PathBuf {
    crate::config_dir().join("voice")
}

impl Prefs {
    pub fn load() -> Self {
        std::fs::read_to_string(prefs_path()).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    fn parse(text: &str) -> Self {
        let mut prefs = Prefs::default();
        for (name, value) in text.lines().filter_map(|l| l.split_once('=')) {
            match (name.trim(), value.trim()) {
                ("speak", v) => prefs.speak = [Speak::Off, Speak::Asked, Speak::Always].into_iter().find(|s| s.key() == v).unwrap_or_default(),
                ("voice", "espeak") => prefs.voice = Voice::Device,
                ("voice", v) => prefs.voice = speech::openai::VOICES.iter().find(|n| **n == v).map_or_else(Voice::default, |n| Voice::OpenAi(n)),
                _ => {}
            }
        }
        prefs
    }

    pub fn save(&self) -> bool {
        let text = format!("speak={}\nvoice={}\n", self.speak.key(), self.voice.key());
        std::fs::create_dir_all(crate::config_dir()).ok();
        crate::sys::write_atomic(&prefs_path(), text.as_bytes()).is_ok()
    }
}

// ---------------------------------------------------------------- listening ---

/// The ALSA device listened to and spoken through. The handheld's is a
/// dsnoop for capture and dmix for playback, set up with the codec.
const DEVICE: &str = "default";

/// Words with which to say the microphone heard nothing but the room.
pub const NOTHING_HEARD: &str = "heard nothing · mic on?";
/// No key to write it down with (Settings › Keys).
pub const NO_KEY: &str = "add an openai key to talk";

/// Trouble as the island says it: a few words, short enough to sit in it
/// whole at the largest text size. (The full reason, from OpenAI or ALSA,
/// is what `ai --listen` prints in the Terminal.)
pub fn brief(why: &str) -> String {
    let lower = why.to_lowercase();
    let said = [
        ("refused the key", "openai refused the key"),
        ("out of credit", "openai: no credit, or busy"),
        ("no internet", "no internet"),
        ("could not reach", "could not reach openai"),
        ("no answer in time", "openai did not answer"),
        ("secure connection", "secure connection failed"),
        ("is busy", "openai is busy · try again"),
        ("cannot use its voice", "this key has no voice models"),
        ("too long to send", "that was too long to send"),
        ("microphone", "the microphone did not open"),
        ("arecord", "the microphone did not open"),
        ("aplay", "the speaker did not open"),
        ("speaker", "the speaker did not open"),
        ("espeak", "espeak did not start"),
    ];
    match said.iter().find(|(seen, _)| lower.contains(seen)) {
        Some((_, short)) => (*short).to_owned(),
        None if why.chars().count() <= 26 => why.to_owned(),
        None => "openai could not do it".to_owned(),
    }
}

enum Heard {
    Sound(Vec<i16>),
    Failed(String),
    Ended,
}

/// The microphone open, gathering what it hears (on a thread of its own).
pub struct Listener {
    samples: Vec<i16>,
    rx: Receiver<Heard>,
    procs: Procs,
    stop: Arc<AtomicBool>,
    failed: Option<String>,
    level: f32,
    ended: bool,
}

/// Where the sound comes from: the microphone; on a computer, the WAV file
/// `CATALYST_VOICE_WAV` names, or else the test signal.
fn microphone(procs: &Procs) -> Result<Box<dyn Source>, String> {
    if let Some(path) = std::env::var_os("CATALYST_VOICE_WAV") {
        return Ok(Box::new(speech::source::Paced::wav_file(std::path::Path::new(&path))?));
    }
    if crate::ai::demo() {
        return Ok(Box::new(speech::source::Paced::test_signal()));
    }
    Ok(Box::new(speech::source::Arecord::start(DEVICE, procs)?))
}

/// Open the microphone and start gathering what it hears.
pub fn listen() -> Listener {
    let (tx, rx) = mpsc::channel();
    let procs = Procs::default();
    let stop = Arc::new(AtomicBool::new(false));
    let (thread_procs, thread_stop) = (procs.clone(), stop.clone());
    let started = std::thread::Builder::new().name("voice-listen".into()).stack_size(256 * 1024).spawn(move || {
        let mut source = match microphone(&thread_procs) {
            Ok(source) => source,
            Err(why) => {
                tx.send(Heard::Failed(why)).ok();
                return;
            }
        };
        while !thread_stop.load(Ordering::Relaxed) {
            let mut chunk = Vec::new();
            match source.read(&mut chunk) {
                Ok(more) => {
                    if !chunk.is_empty() && tx.send(Heard::Sound(chunk)).is_err() {
                        break;
                    }
                    if !more {
                        break;
                    }
                }
                Err(why) => {
                    tx.send(Heard::Failed(why)).ok();
                    break;
                }
            }
        }
        tx.send(Heard::Ended).ok();
    });
    let mut listener = Listener { samples: Vec::new(), rx, procs, stop, failed: None, level: 0.0, ended: false };
    if started.is_err() {
        listener.failed = Some("could not start listening".into());
    }
    listener
}

impl Listener {
    /// Take in what has arrived since the last look.
    pub fn take_in(&mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(Heard::Sound(chunk)) => {
                    self.level = speech::level(&chunk);
                    self.samples.extend(chunk);
                }
                Ok(Heard::Failed(why)) => self.failed = Some(why),
                Ok(Heard::Ended) | Err(TryRecvError::Disconnected) => {
                    self.ended = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
    }

    /// How loud it is just now, 0 to 1.
    pub fn level(&self) -> f32 {
        self.level
    }

    /// How long it has heard for. (For apps with a flow of their own.)
    #[allow(dead_code)]
    pub fn seconds(&self) -> f32 {
        self.samples.len() as f32 / speech::RATE as f32
    }

    /// Why it stopped hearing, if it did.
    pub fn failed(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    /// Close the microphone and hand over what it heard. Waits a moment for
    /// the last of it: call it off the shell's thread.
    pub fn finish(mut self) -> Result<Clip, String> {
        self.close();
        let until = Instant::now() + Duration::from_millis(400);
        while !self.ended && Instant::now() < until {
            match self.rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Heard::Sound(chunk)) => self.samples.extend(chunk),
                Ok(Heard::Failed(why)) => self.failed = Some(why),
                Ok(Heard::Ended) | Err(mpsc::RecvTimeoutError::Disconnected) => self.ended = true,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        match (self.failed.take(), self.samples.is_empty()) {
            (Some(why), true) => Err(why),
            _ => Ok(Clip { samples: std::mem::take(&mut self.samples) }),
        }
    }

    fn close(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.procs.kill_all();
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.close();
    }
}

/// Who writes down what was said.
enum Hearing {
    OpenAi(String),
    /// The simulator's script.
    Demo,
}

fn hearing() -> Result<Hearing, String> {
    if crate::ai::demo() {
        return Ok(Hearing::Demo);
    }
    Keys::load().get("openai").map(|k| Hearing::OpenAi(k.to_owned())).ok_or_else(|| NO_KEY.to_owned())
}

/// Whether anything can write down what is said: the OpenAI key is set.
pub fn can_hear() -> Result<(), String> {
    hearing().map(|_| ())
}

/// The simulator's transcripts: a question for the assistant, a line for a note.
pub const DEMO_QUESTION: &str = "How is the robot doing?";
pub const DEMO_NOTE: &str = "Swap the intake belt before Q32, and check the CAN wiring on the left side.";

/// What was said in `clip`, as text. Blocks (a request over the internet):
/// call it from a thread. (The shell's push-to-talk calls it through
/// [`Talk`]; an app with a flow of its own, after [`listen`].)
#[allow(dead_code)]
pub fn dictate(clip: &Clip) -> Result<String, String> {
    dictate_for(clip, Dictation::Text)
}

fn dictate_for(clip: &Clip, kind: Dictation) -> Result<String, String> {
    if clip.silent() {
        return Err(NOTHING_HEARD.into());
    }
    match hearing()? {
        Hearing::OpenAi(key) => speech::openai::transcribe(&key, &clip.wav(), speech::openai::KEYWORDS, &crate::config_dir()),
        Hearing::Demo => {
            std::thread::sleep(Duration::from_secs_f32(DEMO_WRITING));
            Ok(if kind == Dictation::Question { DEMO_QUESTION } else { DEMO_NOTE }.to_owned())
        }
    }
}

/// The simulator's script takes about as long as OpenAI does.
const DEMO_WRITING: f32 = 0.8;

// ------------------------------------------------------------ push-to-talk ---

/// A press shorter than this is a tap: nothing is sent.
const TAP: f32 = 0.35;
/// After the key comes up the microphone stays open this long, for the end
/// of the last word (people let go as they finish it). My own tuning.
const TAIL: f32 = 0.15;
/// Listening stops by itself after this long.
const MOST: f32 = 60.0;
/// The meter's bars.
pub const BARS: usize = 5;
/// Room the meter takes in the island, beside the words.
pub const METER_ROOM: f32 = BARS as f32 * 2.0 + (BARS - 1) as f32 * 2.5 + 8.0;

/// Where what is said is going: what the screen in front takes, and which
/// screen that was (it may have changed by the time the words are ready).
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub kind: Dictation,
    pub depth: usize,
    pub title: String,
}

enum Phase {
    Idle,
    Listening { listener: Listener, since: f32, let_go: Option<f32>, target: Target },
    Writing { rx: Receiver<Result<String, String>>, since: f32, target: Target },
}

/// What push-to-talk asks of the shell.
#[derive(Debug, PartialEq)]
pub enum Event {
    /// The key woke the screen, and was spent doing so.
    Woke,
    Say(String),
    /// What was said, written down, for where it was going.
    Heard(String, Target),
}

/// Push-to-talk as the shell holds it: the key, the listening, the writing
/// down, and the island's meter.
pub struct Talk {
    phase: Phase,
    was_down: bool,
    /// This press is used up (it woke the screen, was refused, or was let
    /// go of with Esc): nothing more until the key comes up.
    spent: bool,
    bars: [Animated; BARS],
    /// Recent levels with when they were heard, for the outer bars to
    /// follow a beat behind the middle one.
    history: VecDeque<(f32, f32)>,
    clock: f32,
    note: Option<String>,
    /// The calm version: no wave while the words are written down (the
    /// meter still follows the voice: that is what it says, not decoration).
    pub calm: bool,
    /// What the island shows, set by the gallery to check each state over
    /// each screen without talking.
    #[cfg(any(test, not(target_arch = "aarch64")))]
    pub staged: Option<Island>,
}

impl Talk {
    pub fn new() -> Self {
        Talk {
            phase: Phase::Idle,
            was_down: false,
            spent: false,
            // A value following a held input: Bezel's `hold` spring.
            bars: std::array::from_fn(|_| Animated::new(0.0, role::hold()).with_rest(0.004)),
            history: VecDeque::new(),
            clock: 0.0,
            note: None,
            calm: false,
            #[cfg(any(test, not(target_arch = "aarch64")))]
            staged: None,
        }
    }

    /// Anything under way: listening, writing down, or an answer being said.
    pub fn active(&self) -> bool {
        self.listening() || speaking()
    }

    /// Listening, or writing down what was heard: someone is at the keys.
    pub fn listening(&self) -> bool {
        !matches!(self.phase, Phase::Idle)
    }

    /// The key, held down now, has been used for something else (it woke
    /// the handheld from deep standby): nothing until it comes up.
    pub fn spend(&mut self) {
        self.was_down = true;
        self.spent = true;
    }

    /// The key, and time. `awake`: the screen is on; `here`: where what is
    /// said would go, or None while something is in the way (the start
    /// animation, the tour, the control centre).
    pub fn tick(&mut self, dt: f32, down: bool, awake: bool, here: Option<Target>) -> Option<Event> {
        self.clock += dt;
        let pressed = down && !self.was_down;
        let released = !down && self.was_down;
        self.was_down = down;
        if !down {
            self.spent = false;
        }
        let mut event = self.note.take().map(Event::Say);
        if pressed && !self.spent {
            event = self.press(awake, here).or(event);
        }
        let now = self.clock;
        let mut level = 0.0;
        match &mut self.phase {
            Phase::Idle => {}
            Phase::Listening { listener, since, let_go, target } => {
                listener.take_in();
                level = listener.level();
                if let Some(why) = listener.failed().map(brief) {
                    self.phase = Phase::Idle;
                    self.spent = down;
                    return Some(Event::Say(why));
                }
                if released && let_go.is_none() {
                    if now - *since < TAP {
                        self.phase = Phase::Idle;
                        return Some(Event::Say("hold alt while you talk".into()));
                    }
                    *let_go = Some(now);
                }
                // Long enough: it stops by itself, and the key held on is spent.
                if let_go.is_none() && now - *since >= MOST {
                    *let_go = Some(now - TAIL);
                    self.spent = true;
                }
                if let_go.is_some_and(|at| now - at >= TAIL) {
                    let target = target.clone();
                    let Phase::Listening { listener, .. } = std::mem::replace(&mut self.phase, Phase::Idle) else { unreachable!() };
                    self.phase = Phase::Writing { rx: write_down(listener, target.kind), since: now, target };
                }
            }
            Phase::Writing { rx, target, .. } => match rx.try_recv() {
                Ok(Ok(text)) => {
                    let target = target.clone();
                    self.phase = Phase::Idle;
                    let text = text.trim().to_owned();
                    return Some(if text.is_empty() { Event::Say("did not catch that".into()) } else { Event::Heard(text, target) });
                }
                Ok(Err(why)) => {
                    self.phase = Phase::Idle;
                    return Some(Event::Say(brief(&why)));
                }
                Err(TryRecvError::Disconnected) => {
                    self.phase = Phase::Idle;
                    return Some(Event::Say("could not write it down".into()));
                }
                Err(TryRecvError::Empty) => {}
            },
        }
        if let Some(why) = speaking_failed() {
            event = event.or(Some(Event::Say(brief(&why))));
        }
        self.meter(dt, level);
        event
    }

    /// The key has just gone down.
    fn press(&mut self, awake: bool, here: Option<Target>) -> Option<Event> {
        if !awake {
            self.spent = true;
            return Some(Event::Woke);
        }
        let Some(target) = here else {
            self.spent = true;
            return None;
        };
        if let Err(why) = can_hear() {
            self.spent = true;
            return Some(Event::Say(why));
        }
        // Talking over an answer being said stops it; talking again while
        // the last is being written down starts over.
        stop_speaking();
        self.history.clear();
        self.phase = Phase::Listening { listener: listen(), since: self.clock, let_go: None, target };
        None
    }

    /// A key while something is under way. True if it was spent here: Esc
    /// stops an answer being said, or lets go of what is being listened to
    /// or written down. Any other key stops the answer and goes on as ever.
    pub fn key(&mut self, key: Key) -> bool {
        let esc = matches!(key, Key::Back | Key::Leave);
        if speaking() {
            stop_speaking();
            if esc {
                return true;
            }
        }
        if esc && !matches!(self.phase, Phase::Idle) {
            self.note = Some(if matches!(self.phase, Phase::Listening { .. }) { "stopped listening".into() } else { "let it go".into() });
            self.phase = Phase::Idle;
            self.spent = self.was_down;
            return true;
        }
        false
    }

    /// The bars, following the level (the middle one now, the outer ones a
    /// beat behind), or a quiet wave while the words are written down.
    fn meter(&mut self, dt: f32, level: f32) {
        let now = self.clock;
        let level = match &self.phase {
            Phase::Listening { .. } => Some(level),
            _ => speaking_level(),
        };
        let targets: [f32; BARS] = match (&self.phase, level) {
            (Phase::Writing { .. }, _) if self.calm => [0.28; BARS],
            (Phase::Writing { since, .. }, _) => std::array::from_fn(|i| 0.22 + 0.14 * (0.5 + 0.5 * ((now - since) * 7.0 - i as f32 * 0.9).sin())),
            (_, Some(level)) => {
                self.history.push_back((now, level));
                while self.history.front().is_some_and(|(at, _)| now - at > 0.4) {
                    self.history.pop_front();
                }
                let at = |ago: f32| self.history.iter().rev().find(|(t, _)| now - t >= ago).or(self.history.front()).map_or(0.0, |(_, l)| *l);
                // Middle, its neighbours 60 ms behind, the outer pair 120 ms.
                const SHAPE: [(f32, f32); BARS] = [(0.12, 0.62), (0.06, 0.84), (0.0, 1.0), (0.06, 0.84), (0.12, 0.62)];
                SHAPE.map(|(ago, scale)| at(ago) * scale)
            }
            _ => [0.0; BARS],
        };
        for (bar, to) in self.bars.iter_mut().zip(targets) {
            bar.set(to);
            bar.tick(dt);
        }
    }

    /// What the island shows, while there is something to.
    pub fn island(&self) -> Option<Island> {
        #[cfg(any(test, not(target_arch = "aarch64")))]
        if let Some(staged) = &self.staged {
            return Some(staged.clone());
        }
        let bars = self.bars.each_ref().map(|b| b.value().clamp(0.0, 1.0));
        match &self.phase {
            Phase::Listening { target, .. } => Some(Island {
                icon: "mic",
                hot: true,
                // The live microphone and its meter say it listens; the words
                // say what letting go does (short, so the title keeps its room).
                words: if target.kind == Dictation::Question { "let go to ask" } else { "let go to type" }.into(),
                bars,
            }),
            Phase::Writing { .. } => Some(Island { icon: "mic", hot: false, words: "writing it down".into(), bars }),
            Phase::Idle if !speaking() => None,
            Phase::Idle => SPEAKING.with(|s| {
                s.borrow().as_ref().map(|speech| Island {
                    icon: "volume_up",
                    hot: false,
                    words: if speech.ai_voice { "ai voice · any key stops" } else { "speaking · any key stops" }.into(),
                    bars,
                })
            }),
        }
    }
}

/// The island while talking: an icon, words, the meter.
#[derive(Clone, Debug)]
pub struct Island {
    pub icon: &'static str,
    /// The microphone is live: its icon is the signal colour.
    pub hot: bool,
    pub words: String,
    pub bars: [f32; BARS],
}

/// Write down what a listener heard, on a thread of its own.
fn write_down(listener: Listener, kind: Dictation) -> Receiver<Result<String, String>> {
    let (tx, rx) = mpsc::channel();
    let started = std::thread::Builder::new().name("voice-write".into()).stack_size(256 * 1024).spawn(move || {
        let heard = listener.finish().and_then(|clip| dictate_for(&clip, kind));
        tx.send(heard).ok();
    });
    if started.is_err() {
        let (tx, rx) = mpsc::channel();
        tx.send(Err("could not start writing it down".into())).ok();
        return rx;
    }
    rx
}

/// The island's icon, words and meter, on its glass (`words` fading in as
/// the island grows, as any message's do).
pub fn draw_island(g: &mut Gfx, island: Rect, talk: &Island, label: &str, words: f32) {
    let t = g.tokens;
    let caption = g.ty.caption;
    let cy = island.cy();
    let icon_color = if talk.hot { t.signal } else { t.ink };
    g.icon_alpha(talk.icon, talk.hot, island.x + 12.0, cy, 13.0, icon_color, words.max(0.35));
    g.text_alpha(island.x + 22.0, cy, label, caption, t.ink, words);
    // The meter: capsule bars at the right end, rising from their middle.
    let most = (island.h - 8.0).max(4.0);
    let mut x = island.right() - 10.0 - (METER_ROOM - 8.0);
    for level in talk.bars {
        let h = (2.0 + (most - 2.0) * level).round().max(2.0);
        g.canvas.rrect(Rect::new(x, cy - h * 0.5, 2.0, h), 1.0, g.good(), words.max(0.35));
        x += 4.5;
    }
}

// ----------------------------------------------------------- spoken answers ---

/// An answer of more words than this is said in a couple of sentences.
const SPOKEN_WORDS: usize = 45;
/// What espeak says of a long one: its first sentences, up to this many words.
const LEAD_WORDS: usize = 35;

/// An answer being said: what stops it, and the levels for the meter.
struct Speech {
    stop: Arc<AtomicBool>,
    procs: Procs,
    played: Arc<Mutex<Played>>,
    /// An AI voice (OpenAI's), which the island says.
    ai_voice: bool,
}

#[derive(Default)]
struct Played {
    started: Option<Instant>,
    /// One level for each 20 ms of sound given to the speaker.
    levels: Vec<f32>,
    rate: u32,
    pending: Vec<i16>,
    done: bool,
    failed: Option<String>,
}

thread_local! {
    /// The answer being said, if one is (on the shell's thread, as the key).
    static SPEAKING: RefCell<Option<Speech>> = const { RefCell::new(None) };
}

/// Whether an answer is being said. One said to its end is let go here; one
/// that failed waits for [`speaking_failed`] to tell why.
pub fn speaking() -> bool {
    SPEAKING.with(|s| {
        let mut s = s.borrow_mut();
        let state = s.as_ref().map(|speech| speech.played.lock().map_or((true, false), |p| (p.done, p.failed.is_some())));
        match state {
            None => false,
            Some((false, _)) => true,
            Some((true, failed)) => {
                if !failed {
                    *s = None;
                }
                false
            }
        }
    })
}

/// Why the answer could not be said, once, when it ended so.
fn speaking_failed() -> Option<String> {
    SPEAKING.with(|s| {
        let failed = s.borrow().as_ref().and_then(|speech| speech.played.lock().ok().and_then(|mut p| if p.done { p.failed.take() } else { None }));
        if failed.is_some() {
            *s.borrow_mut() = None;
        }
        failed
    })
}

/// How loud what is being said is just now.
fn speaking_level() -> Option<f32> {
    SPEAKING.with(|s| {
        let speech = s.borrow();
        let played = speech.as_ref()?.played.lock().ok()?;
        let Some(started) = played.started else { return Some(0.0) };
        let at = (started.elapsed().as_secs_f32() / 0.02) as usize;
        Some(played.levels.get(at).copied().unwrap_or(0.0))
    })
}

/// Stop the answer being said, at once.
pub fn stop_speaking() {
    SPEAKING.with(|s| {
        if let Some(speech) = s.borrow_mut().take() {
            speech.stop.store(true, Ordering::Relaxed);
            speech.procs.kill_all();
        }
    });
}

/// An answer to say, if answers are said: `asked_aloud` when its question
/// was spoken. Returns at once; the saying happens on a thread.
pub fn answer(text: &str, asked_aloud: bool) {
    answer_with(text, asked_aloud, Prefs::load());
}

fn answer_with(text: &str, asked_aloud: bool, prefs: Prefs) {
    let wanted = match prefs.speak {
        Speak::Off => false,
        Speak::Asked => asked_aloud,
        Speak::Always => true,
    };
    if wanted {
        if let Err(why) = say(text, prefs.voice) {
            // Said in the island at the next look, like any other trouble.
            SPEAKING.with(|s| {
                let played = Played { done: true, failed: Some(why), ..Played::default() };
                *s.borrow_mut() = Some(Speech { stop: Arc::new(AtomicBool::new(false)), procs: Procs::default(), played: Arc::new(Mutex::new(played)), ai_voice: false });
            });
        }
    }
}

/// Who says it.
enum Speaker {
    OpenAi { key: String, voice: &'static str },
    Espeak,
    /// The simulator: a test signal as long as saying it would take.
    Demo,
}

fn speaker(voice: Voice) -> Result<Speaker, String> {
    if crate::ai::demo() {
        return Ok(Speaker::Demo);
    }
    let espeak = || speech::espeak::available().then_some(Speaker::Espeak);
    match voice {
        Voice::OpenAi(name) => match Keys::load().get("openai") {
            Some(key) => Ok(Speaker::OpenAi { key: key.to_owned(), voice: name }),
            None => espeak().ok_or_else(|| "add an openai key to speak".into()),
        },
        Voice::Device => espeak().ok_or_else(|| "espeak is not installed".into()),
    }
}

/// Say `text` with `voice`, on a thread; any answer being said stops first.
pub fn say(text: &str, voice: Voice) -> Result<(), String> {
    stop_speaking();
    let speaker = speaker(voice)?;
    let spoken = for_speech(text);
    if spoken.is_empty() {
        return Ok(());
    }
    let long = spoken.split_whitespace().count() > SPOKEN_WORDS;
    let stop = Arc::new(AtomicBool::new(false));
    let procs = Procs::default();
    let played = Arc::new(Mutex::new(Played::default()));
    let ai_voice = matches!(speaker, Speaker::OpenAi { .. } | Speaker::Demo);
    let (t_stop, t_procs, t_played) = (stop.clone(), procs.clone(), played.clone());
    std::thread::Builder::new()
        .name("voice-speak".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            let mut sink: Option<Box<dyn Sink>> = None;
            let result = {
                let mut emit = |rate: u32, samples: &[i16]| -> Result<(), String> {
                    if t_stop.load(Ordering::Relaxed) {
                        return Err(String::new());
                    }
                    if sink.is_none() {
                        sink = Some(speaker_sink(rate, &t_procs)?);
                        if let Ok(mut p) = t_played.lock() {
                            p.started = Some(Instant::now());
                            p.rate = rate;
                        }
                    }
                    if let Ok(mut p) = t_played.lock() {
                        p.note(samples);
                    }
                    sink.as_mut().expect("made above").write(samples)
                };
                match &speaker {
                    Speaker::OpenAi { key, voice } => {
                        let saying = speech::openai::Saying { key, text: &spoken, voice, summarise: long, dir: &crate::config_dir() };
                        speech::openai::speak(&saying, &t_procs, &t_stop, &mut emit)
                    }
                    Speaker::Espeak => speech::espeak::speak(&if long { lead(&spoken) } else { spoken.clone() }, &t_procs, &t_stop, &mut emit),
                    Speaker::Demo => demo_speak(&spoken, long, &t_stop, &mut emit),
                }
            };
            if let Some(sink) = sink.as_mut() {
                sink.finish();
            }
            t_procs.wait_all(&t_stop);
            if let Ok(mut p) = t_played.lock() {
                p.done = true;
                // Stopped on purpose is no failure.
                p.failed = result.err().filter(|why| !why.is_empty() && !t_stop.load(Ordering::Relaxed));
            }
        })
        .map_err(|_| "could not start speaking".to_owned())?;
    SPEAKING.with(|s| *s.borrow_mut() = Some(Speech { stop, procs, played, ai_voice }));
    Ok(())
}

impl Played {
    /// Note the levels of what is about to play, 20 ms at a time.
    fn note(&mut self, samples: &[i16]) {
        let per = (self.rate / 50).max(1) as usize;
        self.pending.extend_from_slice(samples);
        while self.pending.len() >= per {
            let chunk: Vec<i16> = self.pending.drain(..per).collect();
            self.levels.push(speech::level(&chunk));
        }
    }
}

/// The speaker: the handheld's through aplay; a computer's stand-in.
fn speaker_sink(rate: u32, procs: &Procs) -> Result<Box<dyn Sink>, String> {
    if crate::ai::demo() || !cfg!(target_os = "linux") {
        return Ok(Box::new(speech::sink::Paced::new(rate)));
    }
    Ok(Box::new(speech::sink::Aplay::start(DEVICE, rate, procs)?))
}

/// The simulator says it with the test signal, for about as long as a
/// voice would take.
fn demo_speak(text: &str, long: bool, stop: &AtomicBool, emit: &mut dyn FnMut(u32, &[i16]) -> Result<(), String>) -> Result<(), String> {
    let words = if long { 30 } else { text.split_whitespace().count() };
    let seconds = (words as f32 / 2.6).clamp(1.2, 12.0);
    let mut signal = speech::source::Paced::test_signal();
    let mut chunk = Vec::new();
    let mut said = 0.0;
    while said < seconds && !stop.load(Ordering::Relaxed) {
        chunk.clear();
        signal.read(&mut chunk)?;
        emit(speech::RATE, &chunk)?;
        said += chunk.len() as f32 / speech::RATE as f32;
    }
    Ok(())
}

/// An answer as it is said: on one line, its dots and links left out.
pub fn for_speech(text: &str) -> String {
    let words: Vec<&str> = text
        .split_whitespace()
        .filter(|w| !w.starts_with("http://") && !w.starts_with("https://"))
        .map(|w| if w == "·" { "," } else { w })
        .collect();
    let mut out = String::new();
    for w in words {
        if w == "," {
            out.push(',');
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    out.replace(" ,", ",").replace(",,", ",").trim_matches(',').trim().to_owned()
}

/// A long answer's first sentences, up to `LEAD_WORDS`, and a word that
/// the rest is on the screen.
pub fn lead(text: &str) -> String {
    let mut out = String::new();
    let mut count = 0;
    for sentence in sentences(text) {
        let n = sentence.split_whitespace().count();
        if count > 0 && count + n > LEAD_WORDS {
            break;
        }
        if count == 0 && n > LEAD_WORDS {
            let cut: Vec<&str> = sentence.split_whitespace().take(LEAD_WORDS).collect();
            out = format!("{}.", cut.join(" ").trim_end_matches([',', ';', ':']));
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(sentence.trim());
        count += n;
    }
    format!("{out} The rest is on the screen.")
}

/// Sentences, each with its stop.
fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    for (i, c) in text.char_indices() {
        if matches!(c, '.' | '!' | '?') && bytes.get(i + 1).is_none_or(|b| b.is_ascii_whitespace()) {
            out.push(text[start..=i].trim());
            start = i + 1;
        }
    }
    if text[start..].trim().len() > 1 {
        out.push(text[start..].trim());
    }
    out
}

// -------------------------------------------------------------- the gallery ---

/// The gallery's voice screens: the island listening over Home, writing it
/// down, the assistant with the question it heard, the answer being said,
/// the voice choices, and a note dictated into.
#[cfg(not(target_arch = "aarch64"))]
pub fn gallery(shell: &mut crate::shell::Shell, dir: &std::path::Path) {
    use crate::platform::Key::*;
    let save = |shell: &mut crate::shell::Shell, name: &str| {
        shell.draw();
        crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}.png")), 3);
    };
    // Frames at the simulator's pace: the microphone's stand-in gives sound
    // as fast as a real one would, so the clock has to pass.
    let run = |shell: &mut crate::shell::Shell, seconds: f32| {
        let until = Instant::now() + Duration::from_secs_f32(seconds);
        while Instant::now() < until {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
            std::thread::sleep(Duration::from_millis(14));
        }
    };
    // Answered by the simulator's script, with a key as the tour's own.
    let prefs = Prefs::load();
    Prefs { speak: Speak::Asked, voice: Voice::default() }.save();
    let held = Keys::load();
    let mut keys = held.clone();
    keys.set("openai", "sk-proj-demo-7f3a");
    keys.save().ok();
    for _ in 0..4 {
        shell.key(Back);
        run(shell, 0.3);
    }
    run(shell, 0.6);
    set_talk_key(true);
    run(shell, 1.3);
    save(shell, "121-talk-listening");
    set_talk_key(false);
    run(shell, 0.3);
    save(shell, "121b-talk-writing");
    // Waited for, not timed: a busy computer runs the script slower.
    let run_until = |shell: &mut crate::shell::Shell, done: &dyn Fn(&crate::shell::Shell) -> bool| {
        let until = Instant::now() + Duration::from_secs(20);
        while !done(shell) && Instant::now() < until {
            run(shell, 0.05);
        }
    };
    run_until(shell, &|s| s.top_title() == "Assistant");
    run(shell, 1.0);
    save(shell, "121c-talk-question");
    run_until(shell, &|_| speaking());
    run(shell, 1.3);
    save(shell, "121d-talk-answer-spoken");
    shell.key(Back);
    run(shell, 0.4);
    shell.key(Tab);
    run(shell, 0.6);
    for _ in 0..6 {
        shell.key(Down);
        run(shell, 0.08);
    }
    run(shell, 0.5);
    save(shell, "121e-assistant-voice-choices");
    shell.key(Back);
    run(shell, 0.4);
    shell.key(Back);
    run(shell, 0.8);
    // A note, dictated into where the cursor is (in a folder of this run's
    // own: other galleries running at the same time clear the tour's).
    let notes = std::env::temp_dir().join(format!("catalyst-voice-gallery-{}", std::process::id()));
    std::fs::create_dir_all(&notes).ok();
    let note = notes.join("2026-11-21-gallery.txt");
    std::fs::write(&note, "Intake\n\n— Q31 · nov 21 10:12\n").ok();
    if let Ok(editor) = crate::files::Editor::open(&note) {
        shell.show(Box::new(editor));
        run(shell, 0.8);
        shell.key(Down);
        run(shell, 0.2);
        set_talk_key(true);
        run(shell, 1.2);
        save(shell, "121f-notes-dictating");
        set_talk_key(false);
        run(shell, 1.4);
        save(shell, "121g-notes-dictated");
        shell.key(Ctrl('q'));
        shell.key(Ctrl('q'));
        run(shell, 0.6);
    }
    std::fs::remove_dir_all(&notes).ok();
    prefs.save();
    held.save().ok();
}

/// Every island push-to-talk shows (and the longest of its messages), over
/// the title of every screen it is most often over (Home, the assistant,
/// Notes, and a note named by its date, the longest title), in both looks
/// at all three text sizes: one sheet per look and size, a strip of the
/// panel's top for each. Returns, for each strip, whether the title was
/// whole and beside the island, or had given way (faded out) under it:
/// never cut short by the island.
#[cfg(not(target_arch = "aarch64"))]
pub fn island_sheets(dir: &std::path::Path) -> Vec<(String, bool)> {
    use crate::gfx::TextSize;
    use crate::look::Theme;
    let bars = [0.45, 0.75, 1.0, 0.7, 0.4];
    let talk = |icon: &'static str, hot: bool, words: &str| Some(Island { icon, hot, words: words.into(), bars });
    let states: Vec<(&str, Option<Island>, Option<&str>)> = vec![
        ("at rest", None, None),
        ("asking", talk("mic", true, "let go to ask"), None),
        ("typing", talk("mic", true, "let go to type"), None),
        ("writing", talk("mic", false, "writing it down"), None),
        ("speaking", talk("volume_up", false, "ai voice · any key stops"), None),
        ("sending", None, Some("sending · any key edits")),
        ("typed", None, Some("typed 15 words")),
        ("no key", None, Some(NO_KEY)),
        // The longest thing push-to-talk says.
        ("trouble", None, Some("the microphone did not open")),
    ];
    // A folder of this run's own: other galleries running at the same time
    // clear the tour's shared one.
    let notes = std::env::temp_dir().join(format!("catalyst-island-sheets-{}", std::process::id()));
    let note = notes.join("2026-11-21-101200.txt");
    let frames = |shell: &mut crate::shell::Shell, n: usize| {
        for _ in 0..n {
            if shell.tick(1.0 / 60.0) {
                shell.draw();
            }
        }
    };
    let mut results = Vec::new();
    for theme in [Theme::Catalyst, Theme::Bezel] {
        for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
            const SCREENS: usize = 4;
            let strip = 30usize;
            let mut sheet = bezel::Canvas::new(crate::gfx::W as i32, (strip * SCREENS * states.len()) as i32);
            for screen in 0..SCREENS {
                let mut shell = crate::shell::Shell::new(crate::robot::Robot::demo());
                shell.settings.theme = theme;
                (shell.settings.wallpaper, shell.settings.accent) = theme.defaults();
                shell.settings.text = size;
                shell.apply_settings();
                frames(&mut shell, 100);
                match screen {
                    1 => shell.show(Box::new(crate::ai::Assistant::new())),
                    2 => shell.show(Box::new(crate::notes::Notes::new())),
                    3 => match std::fs::create_dir_all(&notes).and_then(|_| std::fs::write(&note, "Intake\n\n— Q31 · nov 21 10:12\n")).map_err(|e| e.to_string()).and_then(|_| crate::files::Editor::open(&note)) {
                        Ok(editor) => shell.show(Box::new(editor)),
                        Err(why) => eprintln!("the note did not open: {why}"),
                    },
                    _ => {}
                }
                frames(&mut shell, 90);
                for (k, (name, island, message)) in states.iter().enumerate() {
                    shell.stage_island(island.clone(), *message);
                    frames(&mut shell, 60);
                    shell.draw();
                    let row = (screen * states.len() + k) * strip;
                    let w = crate::gfx::W as usize;
                    sheet.px[row * w..(row + strip) * w].copy_from_slice(&shell.gfx.canvas.px[..strip * w]);
                    let alpha = shell.title_alpha();
                    results.push((format!("{}/{}/{}/{name}", theme.name(), size.name(), shell.top_title()), alpha > 0.99 || alpha < 0.01));
                    shell.stage_island(None, None);
                    frames(&mut shell, 60);
                }
            }
            crate::platform::sim::save_png(&sheet, &dir.join(format!("island-{}-{}.png", theme.name(), size.name())), 2);
        }
    }
    std::fs::remove_dir_all(&notes).ok();
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(kind: Dictation) -> Option<Target> {
        Some(Target { kind, depth: 1, title: "Home".into() })
    }

    /// Run push-to-talk for `seconds` of the clock (and as long in fact:
    /// the microphone's stand-in keeps real time), gathering what it asks.
    fn hold(talk: &mut Talk, down: bool, seconds: f32, here: Option<Target>) -> Vec<Event> {
        let mut events = Vec::new();
        let steps = (seconds / 0.02).ceil() as usize;
        for _ in 0..steps {
            events.extend(talk.tick(0.02, down, true, here.clone()));
            std::thread::sleep(Duration::from_millis(20));
        }
        events
    }

    #[test]
    fn held_it_listens_with_a_live_meter_and_let_go_what_was_said_is_written_down() {
        crate::shell::tests::sandbox();
        let mut talk = Talk::new();
        assert!(talk.island().is_none(), "nothing at rest");
        hold(&mut talk, true, 1.0, target(Dictation::Question));
        let island = talk.island().expect("listening");
        assert_eq!((island.icon, island.hot, island.words.as_str()), ("mic", true, "let go to ask"));
        assert!(island.bars.iter().any(|b| *b > 0.1), "the meter moves with the voice: {:?}", island.bars);
        assert!(talk.active());
        let events = hold(&mut talk, false, 2.0, target(Dictation::Question));
        assert_eq!(events, [Event::Heard(DEMO_QUESTION.into(), target(Dictation::Question).unwrap())]);
        assert!(talk.island().is_none());
    }

    #[test]
    fn a_tap_sends_nothing_and_esc_lets_go() {
        crate::shell::tests::sandbox();
        let mut talk = Talk::new();
        hold(&mut talk, true, 0.1, target(Dictation::Text));
        assert_eq!(hold(&mut talk, false, 0.1, target(Dictation::Text)), [Event::Say("hold alt while you talk".into())]);
        hold(&mut talk, true, 0.6, target(Dictation::Text));
        assert!(talk.key(Key::Back), "esc is spent letting go");
        assert!(talk.island().is_none());
        // Still held: nothing more until it comes up and goes down again.
        let events = hold(&mut talk, true, 0.3, target(Dictation::Text));
        assert_eq!(events, [Event::Say("stopped listening".into())]);
        assert!(talk.island().is_none());
        assert!(!talk.key(Key::Char('a')), "other keys go on as ever");
    }

    #[test]
    fn a_dark_screen_wakes_and_the_press_is_spent() {
        crate::shell::tests::sandbox();
        let mut talk = Talk::new();
        assert_eq!(talk.tick(0.02, true, false, target(Dictation::Question)), Some(Event::Woke));
        assert_eq!(talk.tick(0.02, true, true, target(Dictation::Question)), None);
        assert!(talk.island().is_none(), "held on after waking, it does not listen");
        // Something in the way (the tour): nothing.
        talk.tick(0.02, false, true, None);
        assert_eq!(talk.tick(0.02, true, true, None), None);
        assert!(talk.island().is_none());
    }

    #[test]
    fn the_voice_choices_are_kept_and_go_round() {
        crate::shell::tests::sandbox();
        assert_eq!(Prefs::default(), Prefs { speak: Speak::Off, voice: Voice::OpenAi("marin") }, "off unless chosen");
        assert_eq!(Prefs::parse("speak=asked\nvoice=cedar\n"), Prefs { speak: Speak::Asked, voice: Voice::OpenAi("cedar") });
        assert_eq!(Prefs::parse("speak=always\nvoice=espeak"), Prefs { speak: Speak::Always, voice: Voice::Device });
        assert_eq!(Prefs::parse("voice=nobody"), Prefs::default());
        let prefs = Prefs { speak: Speak::Always, voice: Voice::Device };
        assert!(prefs.save());
        assert_eq!(Prefs::load(), prefs);
        Prefs::default().save();
        assert_eq!(Speak::Off.next().next().next(), Speak::Off);
        assert_eq!(Voice::default().next(), Voice::OpenAi("cedar"));
        assert_eq!(Voice::default().next().next(), Voice::Device);
        assert_eq!(Voice::Device.next(), Voice::default());
        assert!(Voice::default().about().contains("ai voice"), "an AI voice says it is one");
    }

    #[test]
    fn answers_are_said_whole_when_short_and_led_in_when_long() {
        assert_eq!(for_speech("The robot is fine · ping 2 ms\nsee https://example.com/x"), "The robot is fine, ping 2 ms see");
        let long = "The robot is linked and running well. Two things stand out: the intake reached 38 A of its 40 A limit at 10:09 and then cleared, and CAN bus can_s0 is busy with 14 devices on it. The ping came back 4 of 4 at about 2 ms.";
        assert!(long.split_whitespace().count() > SPOKEN_WORDS);
        assert_eq!(lead(long), "The robot is linked and running well. The rest is on the screen.");
        let one = "word ".repeat(50);
        assert_eq!(lead(&one).split_whitespace().count(), LEAD_WORDS + "The rest is on the screen.".split_whitespace().count());
        assert_eq!(sentences("One. Two? 3.5 V is low! Done"), ["One.", "Two?", "3.5 V is low!", "Done"]);
    }

    #[test]
    fn an_answer_is_said_only_when_chosen_and_any_key_stops_it() {
        crate::shell::tests::sandbox();
        let asked = Prefs { speak: Speak::Asked, voice: Voice::default() };
        answer_with("The robot is fine.", false, asked);
        assert!(!speaking(), "typed questions are not answered aloud");
        answer_with("The robot is fine.", true, asked);
        assert!(speaking());
        let mut talk = Talk::new();
        let mut loudest = 0.0f32;
        for _ in 0..50 {
            talk.tick(0.02, false, true, None);
            loudest = talk.island().map_or(loudest, |i| i.bars.iter().fold(loudest, |m, b| m.max(*b)));
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(loudest > 0.3, "the meter follows what is said: {loudest}");
        let island = talk.island().expect("the island says it speaks");
        assert_eq!((island.icon, island.words.as_str()), ("volume_up", "ai voice · any key stops"));
        assert!(!talk.key(Key::Char('x')), "a letter stops it and still types");
        assert!(!speaking());
        answer_with("The robot is fine.", true, asked);
        assert!(talk.key(Key::Back), "esc only stops it");
        assert!(!speaking());
        answer_with("The robot is fine.", true, Prefs::default());
        assert!(!speaking(), "off unless chosen");
        // Said to its end, it lets go by itself.
        answer_with("Fine.", true, Prefs { speak: Speak::Always, voice: Voice::default() });
        std::thread::sleep(Duration::from_millis(1800));
        assert!(!speaking());
        assert_eq!(talk.tick(0.02, false, true, None), None, "and nothing to say about it");
    }

    // The galleries' own helpers: not in the handheld's build.
    #[cfg(not(target_arch = "aarch64"))]
    #[test]
    fn alt_wakes_it_from_deep_standby_and_that_press_is_spent() {
        let mut shell = home();
        for _ in 0..240 {
            shell.tick(0.5);
        }
        assert_eq!(shell.standby_state(), crate::standby::State::Off, "two minutes untouched: the screen is off");
        shell.deep_for(600.0, crate::power::Reading::default());
        shell.tick(60.0);
        assert!(shell.deep_asleep(), "a minute more: still deep");
        set_talk_key(true);
        shell.tick(60.0);
        assert!(!shell.deep_asleep(), "alt wakes it, as any key does");
        run(&mut shell, 0.6);
        assert_ne!(shell.island_words().as_deref(), Some("let go to ask"), "the waking press does not listen");
        set_talk_key(false);
        run(&mut shell, 0.1);
    }

    // The galleries' own helpers: not in the handheld's build.
    #[cfg(not(target_arch = "aarch64"))]
    #[test]
    fn a_title_gives_way_to_an_island_grown_over_it_and_comes_back_whole() {
        let mut shell = home();
        let frames = |shell: &mut crate::shell::Shell, n: usize| {
            for _ in 0..n {
                if shell.tick(1.0 / 60.0) {
                    shell.draw();
                }
            }
        };
        frames(&mut shell, 30);
        assert!(shell.title_alpha() > 0.99, "at rest, the title shows");
        // Beside "let go to ask", "Catalyst" still fits; beside an answer
        // being said it does not, and fades out rather than being cut.
        let island = |words: &str| Some(Island { icon: "mic", hot: true, words: words.into(), bars: [0.5; BARS] });
        shell.stage_island(island("let go to ask"), None);
        frames(&mut shell, 60);
        assert!(shell.title_alpha() > 0.99);
        shell.stage_island(island("ai voice · any key stops"), None);
        frames(&mut shell, 3);
        let fading = shell.title_alpha();
        assert!(fading < 0.99 && fading > 0.01, "on a spring, not at once: {fading}");
        frames(&mut shell, 60);
        assert!(shell.title_alpha() < 0.01, "gone under the island");
        shell.stage_island(None, None);
        frames(&mut shell, 90);
        assert!(shell.title_alpha() > 0.99, "and back once it has shrunk clear");
    }

    #[test]
    fn trouble_is_said_in_words_the_island_holds_whole() {
        let reasons = [
            "OpenAI refused the key · check it in Settings › Keys",
            "too many requests, or the account is out of credit",
            "no internet: the service's name did not resolve",
            "the secure connection failed · is the clock right?",
            "the microphone did not open: no such file or directory",
            "arecord is not on this handheld",
            "aplay is not on this handheld",
            "A very long message from the service that nobody planned for at all",
            "did not catch that",
        ];
        for why in reasons {
            assert!(brief(why).chars().count() <= 27, "{why} → {}", brief(why));
        }
        assert_eq!(brief("the microphone did not open: no such device"), "the microphone did not open");
        assert_eq!(brief("did not catch that"), "did not catch that", "short ones stay as they are");
        for said in [NO_KEY, NOTHING_HEARD, "hold alt while you talk", "stopped listening", "could not write it down", "add an openai key to speak", "espeak is not installed"] {
            assert!(said.chars().count() <= 27, "{said}");
        }
    }

    #[test]
    fn the_talk_key_is_alt_on_either_side() {
        assert!(is_talk_code(56) && is_talk_code(100));
        assert!(!is_talk_code(29) && !is_talk_code(42), "not Ctrl or Shift");
        set_talk_key(true);
        assert!(talk_key_down());
        set_talk_key(false);
        assert!(!talk_key_down());
    }

    /// The shell, run for `seconds` of the clock and as long in fact.
    fn run(shell: &mut crate::shell::Shell, seconds: f32) {
        for _ in 0..(seconds / 0.02).ceil() as usize {
            if shell.tick(0.02) {
                shell.draw();
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn home() -> crate::shell::Shell {
        crate::shell::tests::sandbox();
        let mut shell = crate::shell::Shell::new(crate::robot::Robot::demo());
        shell.apply_settings();
        run(&mut shell, 0.3);
        shell
    }

    #[test]
    fn held_on_home_the_island_listens_and_the_assistant_grows_out_of_it_asked() {
        let mut shell = home();
        set_talk_key(true);
        run(&mut shell, 0.8);
        assert_eq!(shell.island_words().as_deref(), Some("let go to ask"));
        set_talk_key(false);
        run(&mut shell, 0.4);
        assert_eq!(shell.island_words().as_deref(), Some("writing it down"));
        run(&mut shell, 1.2);
        assert_eq!(shell.top_title(), "Assistant", "opened with the question");
        // Esc there leaves it, back to Home.
        shell.key(Key::Back);
        shell.key(Key::Back);
        run(&mut shell, 0.8);
        assert_eq!(shell.top_title(), "Catalyst");
    }

    #[test]
    fn in_a_note_what_was_said_is_typed_at_the_cursor_and_kept_like_typing() {
        let mut shell = home();
        let dir = std::env::temp_dir().join(format!("catalyst-dictation-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let note = dir.join("intake.txt");
        std::fs::write(&note, "Intake\n\n— Q31\n").unwrap();
        shell.show(Box::new(crate::files::Editor::open(&note).unwrap()));
        run(&mut shell, 0.6);
        shell.key(Key::Down);
        set_talk_key(true);
        run(&mut shell, 0.8);
        assert_eq!(shell.island_words().as_deref(), Some("let go to type"));
        set_talk_key(false);
        run(&mut shell, 1.4);
        assert_eq!(shell.island_words().as_deref(), Some("typed 15 words"));
        assert!(std::fs::read_to_string(&note).unwrap().starts_with("Intake\n\n"), "not saved until saved, as typing");
        shell.key(Key::Ctrl('s'));
        let kept = std::fs::read_to_string(&note).unwrap();
        let lines: Vec<&str> = kept.lines().collect();
        assert_eq!(lines[0], "Intake");
        assert_eq!(lines.last(), Some(&"— Q31"));
        assert_eq!(lines[1..lines.len() - 1].join(" "), DEMO_NOTE, "on the line the cursor was on, broken where the panel ends");
        assert!(lines.iter().all(|l| l.chars().count() < 48), "{lines:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn on_the_notes_list_what_was_said_starts_a_new_note() {
        let mut shell = home();
        let notes = crate::config_dir().join("notes");
        std::fs::create_dir_all(&notes).unwrap();
        let before: Vec<_> = std::fs::read_dir(&notes).unwrap().flatten().map(|e| e.path()).collect();
        shell.show(Box::new(crate::notes::Notes::new()));
        run(&mut shell, 0.6);
        set_talk_key(true);
        run(&mut shell, 0.8);
        set_talk_key(false);
        run(&mut shell, 1.4);
        // Other tests write notes into the same folder meanwhile: only the
        // new ones that hold what was said count.
        let said = |p: &std::path::PathBuf| std::fs::read_to_string(p).is_ok_and(|t| t.split_whitespace().collect::<Vec<_>>().join(" ").starts_with(DEMO_NOTE));
        let new: Vec<_> = std::fs::read_dir(&notes).unwrap().flatten().map(|e| e.path()).filter(|p| !before.contains(p) && said(p)).collect();
        assert_eq!(new.len(), 1, "one new note, with what was said");
        let text = std::fs::read_to_string(&new[0]).unwrap();
        assert!(text.contains("— "), "stamped as any new note is");
        assert_ne!(shell.top_title(), "Notes", "open in the editor, to carry on");
        std::fs::remove_file(&new[0]).ok();
    }

    #[test]
    fn heard_nothing_is_said_so_rather_than_sent() {
        crate::shell::tests::sandbox();
        let quiet = Clip { samples: vec![3; speech::RATE as usize] };
        assert_eq!(dictate(&quiet).unwrap_err(), NOTHING_HEARD);
    }
}
