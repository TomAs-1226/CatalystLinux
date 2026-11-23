//! Voice commands, recognised on the smart camera's NPU (Catalyst
//! Coprocessor), and what each does on the handheld.
//!
//! **The key** is the talk key, Alt (voice.rs), the one key the shell had
//! free. Two gestures on it:
//! - **Hold Alt** and say a word: the audio streams to the camera only while
//!   the key is held; let go, and its keyword spotter (a small CNN on the
//!   K210's KPU, trained on Google's Speech Commands, CC-BY-4.0) answers in
//!   a few milliseconds. A command does its action at once and nothing else
//!   runs. Anything that is not a command (longer than a word, or not sure
//!   enough) is dictation, written down as before, trimmed to the voice the
//!   camera heard.
//! - **Tap Alt** (under a third of a second): dictation, hands-free. It
//!   listens until you stop talking (0.6 s of quiet after the voice, the
//!   camera's voice detector, or the handheld's own when no camera is
//!   there), with no command looked for; Alt again stops it.
//!
//! **The words** ([`LABELS`]): yes, no, up, down, left, right, on, off,
//! stop, go and zero to nine, each with an action chosen in Settings › Voice
//! commands (Team's six calls on one to six, as the number keys send them;
//! the arrows, Enter and Esc; timers; music; apps). The spotter also knows
//! ten other words (bed, bird, cat, dog, happy, house, marvin, sheila, tree,
//! wow) only to tell them apart: they do nothing.
//!
//! **Words of your own** ([`UserWord`]): up to [`USER_WORDS`], each taught by
//! saying it three times; the camera's spotter gives each utterance a 64-d
//! embedding, the word keeps their average, and a later utterance close
//! enough to it (cosine ≥ [`USER_COSINE`]) is that word. Checked before the
//! built-in ones (kws.json: 66% of taught words found at 2% false accepts,
//! measured on unseen speakers; the built-in words 85% at 0.9%).
//!
//! **Power.** The camera wakes for an utterance and idles again; with
//! [`Prefs::ready`] it is kept powered (idle, about 40 mA at 5 V) so the
//! first word is heard at once, otherwise it starts on the key (about 2.5 s,
//! the audio waiting meanwhile) and stays warm 15 s for the next.

use std::path::PathBuf;

use bezel::{Align, Rect};

use crate::gfx::{Gfx, ListState, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::smartcam::speech::Heard;

/// The keyword spotter's labels, in its output order (tools/unitv/speech/kws.json).
pub const LABELS: [&str; 31] = [
    "yes", "no", "up", "down", "left", "right", "on", "off", "stop", "go", "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "bed", "bird", "cat", "dog", "happy", "house", "marvin", "sheila", "tree", "wow", "_silence_",
];
/// The first twenty labels are the commands.
pub const COMMANDS: usize = 20;
/// A command needs this much of the spotter's probability (kws.json's
/// threshold, chosen on the validation speakers: 85% of commands taken,
/// 0.4% taken as the wrong one, 0.9% of other words taken).
pub const THRESHOLD: f32 = 0.75;
/// A taught word needs this cosine to its average (kws.json).
pub const USER_COSINE: f32 = 0.62;
pub const USER_WORDS: usize = 4;
/// Times a word is said to teach it.
pub const TEACH_TIMES: usize = 3;
/// Speech longer than this is not one word: dictation.
pub const COMMAND_MAX_MS: u32 = 1600;

// ----------------------------------------------------------------- actions ---

/// What a word does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Nothing,
    /// As the key pressed.
    Key(KeyName),
    Home,
    /// Team's canned call (1 to 6), as its number key sends it.
    TeamCall(u8),
    /// A timer of so many minutes, in Clock.
    Timer(u32),
    /// Every running timer stopped (and an answer being said).
    StopTimers,
    /// Music: play or pause, the next song.
    Play,
    Pause,
    Next,
    Assistant,
    /// An app, by its name on Home.
    Open(&'static str),
}

/// The keys a word can stand for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyName {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Back,
}

impl KeyName {
    pub fn key(self) -> Key {
        match self {
            KeyName::Up => Key::Up,
            KeyName::Down => Key::Down,
            KeyName::Left => Key::Left,
            KeyName::Right => Key::Right,
            KeyName::Enter => Key::Enter,
            KeyName::Back => Key::Back,
        }
    }
}

/// Every action Settings offers, in the order it steps through them.
pub const CHOICES: [Action; 25] = [
    Action::Nothing,
    Action::Key(KeyName::Up),
    Action::Key(KeyName::Down),
    Action::Key(KeyName::Left),
    Action::Key(KeyName::Right),
    Action::Key(KeyName::Enter),
    Action::Key(KeyName::Back),
    Action::Home,
    Action::TeamCall(1),
    Action::TeamCall(2),
    Action::TeamCall(3),
    Action::TeamCall(4),
    Action::TeamCall(5),
    Action::TeamCall(6),
    Action::Timer(1),
    Action::Timer(5),
    Action::Timer(10),
    Action::StopTimers,
    Action::Play,
    Action::Pause,
    Action::Next,
    Action::Assistant,
    Action::Open("Camera"),
    Action::Open("Team"),
    Action::Open("Clock"),
];

impl Action {
    /// In a few words, for Settings and the island.
    pub fn words(self) -> String {
        match self {
            Action::Nothing => "nothing".into(),
            Action::Key(k) => format!(
                "{} key",
                match k {
                    KeyName::Up => "↑",
                    KeyName::Down => "↓",
                    KeyName::Left => "←",
                    KeyName::Right => "→",
                    KeyName::Enter => "enter",
                    KeyName::Back => "esc",
                }
            ),
            Action::Home => "home".into(),
            Action::TeamCall(n) => format!("team call {n}"),
            Action::Timer(m) => format!("timer {m} min"),
            Action::StopTimers => "stop timers".into(),
            Action::Play => "play music".into(),
            Action::Pause => "pause music".into(),
            Action::Next => "next song".into(),
            Action::Assistant => "assistant".into(),
            Action::Open(app) => format!("open {}", app.to_lowercase()),
        }
    }

    fn key_name(self) -> String {
        match self {
            Action::Open(app) => format!("open:{app}"),
            Action::TeamCall(n) => format!("team:{n}"),
            Action::Timer(m) => format!("timer:{m}"),
            other => other.words(),
        }
    }

    fn from_key_name(s: &str) -> Option<Action> {
        CHOICES.iter().copied().find(|a| a.key_name() == s)
    }

    /// The next (or previous) choice.
    pub fn step(self, by: i32) -> Action {
        let at = CHOICES.iter().position(|a| *a == self).unwrap_or(0) as i32;
        CHOICES[(at + by).rem_euclid(CHOICES.len() as i32) as usize]
    }
}

/// What each command word does unless chosen otherwise.
pub fn default_action(word: &str) -> Action {
    match word {
        "yes" => Action::Key(KeyName::Enter),
        "no" => Action::Key(KeyName::Back),
        "up" => Action::Key(KeyName::Up),
        "down" => Action::Key(KeyName::Down),
        "left" => Action::Key(KeyName::Left),
        "right" => Action::Key(KeyName::Right),
        "on" => Action::Play,
        "off" => Action::Pause,
        "stop" => Action::StopTimers,
        "go" => Action::Assistant,
        "zero" => Action::Home,
        "one" => Action::TeamCall(1),
        "two" => Action::TeamCall(2),
        "three" => Action::TeamCall(3),
        "four" => Action::TeamCall(4),
        "five" => Action::TeamCall(5),
        "six" => Action::TeamCall(6),
        "seven" => Action::Open("Camera"),
        "eight" => Action::Open("Team"),
        "nine" => Action::Timer(5),
        _ => Action::Nothing,
    }
}

// ------------------------------------------------------------------- prefs ---

/// Where dictation is written down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Engine {
    /// Whisper on the handheld when its model is there, else OpenAI's.
    #[default]
    Auto,
    /// Whisper tiny.en on the handheld (whisper_local.rs), offline.
    Local,
    OpenAi,
}

impl Engine {
    pub fn words(self) -> &'static str {
        match self {
            Engine::Auto => "this handheld, else openai",
            Engine::Local => "this handheld only",
            Engine::OpenAi => "openai",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Engine::Auto => "auto",
            Engine::Local => "local",
            Engine::OpenAi => "openai",
        }
    }
    pub fn next(self) -> Engine {
        match self {
            Engine::Auto => Engine::Local,
            Engine::Local => Engine::OpenAi,
            Engine::OpenAi => Engine::Auto,
        }
    }
}

/// A word the user taught: the action it does, and its average embedding.
#[derive(Clone, Debug, PartialEq)]
pub struct UserWord {
    pub action: Action,
    pub mean: Vec<f32>,
}

/// Settings › Voice commands, in `config_dir()/voice-commands.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    /// Commands looked for when the talk key is held (on the smart camera).
    pub enabled: bool,
    /// The camera kept powered and idle, for the first word at once.
    pub ready: bool,
    pub dictation: Engine,
    /// Commands' actions chosen other than the defaults.
    pub actions: Vec<(String, Action)>,
    pub user: Vec<UserWord>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { enabled: true, ready: false, dictation: Engine::Auto, actions: Vec::new(), user: Vec::new() }
    }
}

fn prefs_path() -> PathBuf {
    crate::config_dir().join("voice-commands.json")
}

impl Prefs {
    pub fn load() -> Prefs {
        std::fs::read_to_string(prefs_path()).ok().and_then(|t| Prefs::parse(&t)).unwrap_or_default()
    }

    pub fn parse(text: &str) -> Option<Prefs> {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        let mut p = Prefs { enabled: v["enabled"].as_bool().unwrap_or(true), ready: v["ready"].as_bool().unwrap_or(false), ..Prefs::default() };
        p.dictation = [Engine::Auto, Engine::Local, Engine::OpenAi].into_iter().find(|e| v["dictation"].as_str() == Some(e.key())).unwrap_or_default();
        if let Some(a) = v["actions"].as_object() {
            for (word, act) in a {
                if let Some(act) = act.as_str().and_then(Action::from_key_name) {
                    p.actions.push((word.clone(), act));
                }
            }
        }
        for w in v["user"].as_array().into_iter().flatten() {
            let action = w["action"].as_str().and_then(Action::from_key_name);
            let mean: Vec<f32> = w["mean"].as_array().map(|a| a.iter().filter_map(|x| x.as_f64().map(|x| x as f32)).collect()).unwrap_or_default();
            if let (Some(action), 64) = (action, mean.len()) {
                p.user.push(UserWord { action, mean });
            }
        }
        Some(p)
    }

    pub fn json(&self) -> String {
        let actions: serde_json::Map<String, serde_json::Value> = self.actions.iter().map(|(w, a)| (w.clone(), serde_json::Value::from(a.key_name()))).collect();
        let user: Vec<serde_json::Value> = self.user.iter().map(|u| serde_json::json!({ "action": u.action.key_name(), "mean": u.mean })).collect();
        serde_json::json!({ "enabled": self.enabled, "ready": self.ready, "dictation": self.dictation.key(), "actions": actions, "user": user }).to_string()
    }

    pub fn save(&self) -> bool {
        std::fs::create_dir_all(crate::config_dir()).ok();
        crate::sys::write_atomic(&prefs_path(), self.json().as_bytes()).is_ok()
    }

    /// What `word` does.
    pub fn action(&self, word: &str) -> Action {
        self.actions.iter().find(|(w, _)| w == word).map_or_else(|| default_action(word), |(_, a)| *a)
    }

    pub fn set_action(&mut self, word: &str, action: Action) {
        self.actions.retain(|(w, _)| w != word);
        if action != default_action(word) {
            self.actions.push((word.to_owned(), action));
        }
    }

    /// Whether commands go to the smart camera now.
    pub fn active(&self) -> bool {
        self.enabled && crate::smartcam::speech::available()
    }
}

// ----------------------------------------------------------------- decide ---

/// An embedding scaled to length one.
pub fn unit(v: &[f32]) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n <= f32::EPSILON {
        return v.to_vec();
    }
    v.iter().map(|x| x / n).collect()
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (a, b) = (unit(a), unit(b));
    a.iter().zip(&b).map(|(x, y)| x * y).sum()
}

/// The average of a word's utterances (each made length one first), as
/// length one: what a later utterance is compared with.
pub fn prototype(samples: &[Vec<f32>]) -> Vec<f32> {
    let Some(first) = samples.first() else { return Vec::new() };
    let mut sum = vec![0.0f32; first.len()];
    for s in samples.iter().filter(|s| s.len() == first.len()) {
        for (acc, v) in sum.iter_mut().zip(unit(s)) {
            *acc += v;
        }
    }
    unit(&sum)
}

/// What an utterance was.
#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    /// A word, its action, how sure.
    Command { word: String, action: Action, score: f32 },
    /// Not a command: dictation (if anything was said).
    Speech,
    /// Nothing but the room.
    Silence,
}

/// The rule (kws.json): longer than a word, dictation; then a taught word
/// (cosine to its average ≥ [`USER_COSINE`]); then a built-in command (its
/// probability ≥ [`THRESHOLD`]); then silence if the spotter says so;
/// else dictation.
pub fn decide(heard: &Heard, prefs: &Prefs) -> Decision {
    if heard.vad.state == 0 && heard.top.first().is_none_or(|(i, _)| *i == LABELS.len() - 1) {
        return Decision::Silence;
    }
    if heard.vad.speech_ms > COMMAND_MAX_MS {
        return Decision::Speech;
    }
    if heard.embedding.len() == 64 {
        let best = prefs.user.iter().enumerate().map(|(i, u)| (i, cosine(&heard.embedding, &u.mean))).max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, c)) = best.filter(|(_, c)| *c >= USER_COSINE) {
            let u = &prefs.user[i];
            return Decision::Command { word: format!("your word {}", i + 1), action: u.action, score: c };
        }
    }
    match heard.top.first() {
        Some(&(i, p)) if i < COMMANDS && p >= THRESHOLD => {
            let word = LABELS[i];
            Decision::Command { word: word.to_owned(), action: prefs.action(word), score: p }
        }
        Some(&(i, p)) if i == LABELS.len() - 1 && p >= 0.5 && heard.vad.speech_ms < 200 => Decision::Silence,
        _ => Decision::Speech,
    }
}

// ----------------------------------------------------------------- perform ---

/// What the shell does for a command (what is not done here).
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// Done; the island says this.
    Said(String),
    Key(Key),
    Home,
    Assistant,
    /// An app, by its name on Home.
    Open(&'static str),
}

/// A command carried out, as far as it is not the shell's: Team's calls,
/// timers and music here; keys, Home, the assistant and apps for the shell.
pub fn perform(action: Action) -> Act {
    match action {
        Action::Nothing => Act::Said("that word does nothing · settings › voice commands".into()),
        Action::Key(k) => Act::Key(k.key()),
        Action::Home => Act::Home,
        Action::Assistant => Act::Assistant,
        Action::Open(app) => Act::Open(app),
        Action::TeamCall(n) => {
            if !crate::team::member() {
                return Act::Said("not in a team · team call not sent".into());
            }
            let calls = crate::team::calls();
            let Some((words, urgent)) = calls.get(usize::from(n.saturating_sub(1))).cloned() else { return Act::Said(format!("no team call {n}")) };
            Act::Said(match crate::team::send(crate::team::wire::Body::Call(words.clone()), urgent) {
                Ok(()) => format!("sent to the team: {words}"),
                Err(why) => why,
            })
        }
        Action::Timer(min) => {
            crate::clock::add_timer(f64::from(min) * 60.0);
            Act::Said(format!("timer · {min} min"))
        }
        Action::StopTimers => {
            crate::voice::stop_speaking();
            let running = crate::clock::running_timers();
            for (id, ..) in &running {
                crate::clock::remove_timer(*id);
            }
            Act::Said(match running.len() {
                0 => "no timer running".into(),
                1 => "timer stopped".into(),
                n => format!("{n} timers stopped"),
            })
        }
        Action::Play | Action::Pause | Action::Next => {
            let what = match action {
                Action::Play => "play",
                Action::Pause => "pause",
                _ => "next",
            };
            Act::Said(match crate::music::control(what) {
                Ok((words, _)) => format!("music: {words}"),
                Err(why) => why,
            })
        }
    }
}

// ------------------------------------------------------- the handheld's VAD ---

/// The voice in `samples` (16 kHz) when no camera is there to say: the
/// camera's own rule (tools/unitv/speech/features.py's VAD on 32 ms
/// frames every 20 ms: the floor from the first frame, at most −45 dB;
/// speech 12 dB over it and over −60 dB; three frames start it, 30 quiet
/// end it) — (state, start ms, end ms, speech ms).
pub fn vad(samples: &[i16]) -> crate::smartcam::speech::Vad {
    const WIN: usize = 512;
    const HOP: usize = 320;
    let mut floor = -45.0f32;
    let (mut state, mut start, mut last, mut speech, mut run) = (0u8, 0usize, -1i64, 0u32, 0u32);
    let mut k = 0usize;
    while k * HOP + WIN <= samples.len() {
        let frame = &samples[k * HOP..k * HOP + WIN];
        let p = frame.iter().map(|s| (f32::from(*s) / 32768.0).powi(2)).sum::<f32>() / WIN as f32 + 1e-10;
        let e = 10.0 * p.log10();
        if k == 0 && e < floor {
            floor = e;
        }
        let sp = e > floor + 12.0 && e > -60.0;
        if state == 0 {
            run = if sp { run + 1 } else { 0 };
            if run >= 3 {
                state = 1;
                start = (k as i64 - 2 - 5).max(0) as usize;
                last = k as i64;
                speech = 3;
            }
        } else if sp {
            last = k as i64;
            speech += 1;
            state = 1;
        } else if k as i64 - last >= 30 {
            state = 2;
        }
        floor = if e < floor { e } else { floor + 0.05 };
        k += 1;
    }
    let end_ms = if last >= 0 { ((last + 1) * 20 + 12) as u32 } else { 0 };
    crate::smartcam::speech::Vad { state, start_ms: (start * 20) as u32, end_ms, speech_ms: speech * 20 }
}

/// The samples of the voice's span (with a little either side), or none if
/// there was no voice.
pub fn trim(samples: &[i16], v: &crate::smartcam::speech::Vad) -> Vec<i16> {
    if v.state == 0 || v.end_ms <= v.start_ms {
        return Vec::new();
    }
    let at = |ms: u32| (ms as usize * 16).min(samples.len());
    // A fifth of a second after the end: the last word's tail.
    samples[at(v.start_ms)..at(v.end_ms + 200)].to_vec()
}

/// Settings' row, in a few words.
pub fn summary() -> String {
    let p = Prefs::load();
    match (p.enabled, crate::smartcam::chosen()) {
        (false, _) => "off".into(),
        (true, false) => "needs the smart camera".into(),
        (true, true) if p.ready => "on · kept ready".into(),
        (true, true) => "on · hold alt".into(),
    }
}

// -------------------------------------------------------------------- page ---

/// Teaching a word: the utterances heard so far.
struct Teaching {
    slot: usize,
    action: Action,
    heard: Vec<Vec<f32>>,
}

/// Settings › Voice commands.
pub struct CommandsPage {
    prefs: Prefs,
    list: ListState,
    teaching: Option<Teaching>,
    /// An utterance being taught: its session and the answer to come.
    listening: Option<crate::smartcam::speech::Session>,
    waiting: Option<crate::smartcam::speech::Pending>,
    mic: Option<crate::voice::Listener>,
    sent: usize,
    ready_lease: Option<crate::smartcam::Lease>,
    frozen: bool,
}

/// The page's rows, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Enabled,
    Ready,
    Dictation,
    Word(usize),
    User(usize),
}

impl CommandsPage {
    pub fn new() -> CommandsPage {
        CommandsPage { prefs: Prefs::load(), list: ListState::default(), teaching: None, listening: None, waiting: None, mic: None, sent: 0, ready_lease: None, frozen: false }
    }

    /// For the pictures: prefs as given, nothing live.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn with(prefs: Prefs, focus: usize) -> CommandsPage {
        let mut page = CommandsPage { frozen: true, ..CommandsPage::new() };
        page.prefs = prefs;
        page.list.focus = focus;
        page
    }

    fn rows(&self) -> Vec<Row> {
        let mut r = vec![Row::Enabled, Row::Ready, Row::Dictation];
        r.extend((0..COMMANDS).map(Row::Word));
        r.extend((0..USER_WORDS).map(Row::User));
        r
    }

    fn row_text(&self, row: Row) -> (String, String, bool) {
        let p = &self.prefs;
        match row {
            Row::Enabled => ("Commands".into(), if p.enabled { "on · hold alt".into() } else { "off".into() }, false),
            Row::Ready => ("Coprocessor".into(), if p.ready { "kept ready · ~40 mA".into() } else { "wakes on alt · 0 mA".into() }, false),
            Row::Dictation => ("Dictation".into(), p.dictation.words().into(), false),
            Row::Word(i) => (format!("“{}”", LABELS[i]), p.action(LABELS[i]).words(), false),
            Row::User(i) => match (&self.teaching, p.user.get(i)) {
                (Some(t), _) if t.slot == i => (format!("Your word {}", i + 1), format!("say it · {} of {TEACH_TIMES}", t.heard.len() + 1), true),
                (_, Some(u)) => (format!("Your word {}", i + 1), u.action.words(), false),
                (_, None) => (format!("Your word {}", i + 1), "enter teaches one".into(), false),
            },
        }
    }

    fn save(&mut self, cx: &mut Cx) {
        if !self.prefs.save() {
            *cx.say = Some("could not save".into());
        }
        crate::voice::commands_changed();
    }

    /// One utterance of the word being taught: the key held, its audio to
    /// the camera; let go, its embedding.
    fn teach_tick(&mut self, cx: &mut Cx) {
        let down = crate::voice::talk_key_down();
        if self.teaching.is_none() {
            return;
        }
        if down && self.listening.is_none() && self.waiting.is_none() {
            self.listening = Some(crate::smartcam::speech::begin());
            self.mic = Some(crate::voice::listen());
            self.sent = 0;
        }
        if let (Some(session), Some(mic)) = (&self.listening, &mut self.mic) {
            mic.take_in();
            let fresh = mic.since(self.sent);
            session.push(fresh);
            self.sent += fresh.len();
        }
        if !down {
            if let Some(session) = self.listening.take() {
                self.mic = None;
                self.waiting = Some(session.finish(crate::smartcam::speech::Mode::Command));
            }
        }
        let Some(pending) = &self.waiting else { return };
        let Ok(answer) = pending.rx.try_recv() else { return };
        self.waiting = None;
        let Some(t) = self.teaching.as_mut() else { return };
        match answer {
            Ok(h) if h.embedding.len() == 64 && h.vad.state > 0 => {
                t.heard.push(h.embedding);
                if t.heard.len() >= TEACH_TIMES {
                    let word = UserWord { action: t.action, mean: prototype(&t.heard) };
                    let slot = t.slot;
                    if slot < self.prefs.user.len() {
                        self.prefs.user[slot] = word;
                    } else {
                        self.prefs.user.push(word);
                    }
                    self.teaching = None;
                    *cx.say = Some(format!("your word {} · {}", slot + 1, self.prefs.user[slot.min(self.prefs.user.len() - 1)].action.words()));
                    self.save(cx);
                } else {
                    *cx.say = Some(format!("again · {} of {TEACH_TIMES}", t.heard.len() + 1));
                }
            }
            Ok(_) => *cx.say = Some("heard nothing · hold alt and say it".into()),
            Err(why) => {
                self.teaching = None;
                *cx.say = Some(why);
            }
        }
    }
}

impl Screen for CommandsPage {
    fn title(&self) -> &str {
        "Voice commands"
    }

    fn keeps_awake(&self) -> bool {
        self.teaching.is_some()
    }

    /// Teaching takes the talk key itself.
    fn takes_talk_key(&self) -> bool {
        self.teaching.is_some()
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if !self.frozen {
            self.teach_tick(cx);
        }
        Nav::Stay
    }

    fn tick(&mut self, _dt: f32) -> bool {
        self.teaching.is_some()
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let rows = self.rows();
        let row = rows.get(self.list.focus).copied().unwrap_or(Row::Enabled);
        match key {
            Key::Back | Key::Leave if self.teaching.is_some() => {
                (self.teaching, self.listening, self.waiting, self.mic) = (None, None, None, None);
                *cx.say = Some("not taught".into());
            }
            Key::Back | Key::Leave => return Nav::Pop,
            Key::Enter | Key::Right | Key::Left => {
                let by = if key == Key::Left { -1 } else { 1 };
                match row {
                    Row::Enabled => self.prefs.enabled = !self.prefs.enabled,
                    Row::Ready => {
                        self.prefs.ready = !self.prefs.ready;
                        self.ready_lease = None;
                    }
                    Row::Dictation => self.prefs.dictation = self.prefs.dictation.next(),
                    Row::Word(i) => {
                        let now = self.prefs.action(LABELS[i]).step(by);
                        self.prefs.set_action(LABELS[i], now);
                    }
                    Row::User(i) if key == Key::Enter => {
                        if !crate::smartcam::speech::available() {
                            *cx.say = Some("teaching needs the smart camera on the grove port".into());
                            return Nav::Stay;
                        }
                        let action = self.prefs.user.get(i).map_or(Action::Open("Camera"), |u| u.action);
                        self.teaching = Some(Teaching { slot: i.min(self.prefs.user.len()), action, heard: Vec::new() });
                        *cx.say = Some(format!("hold alt and say your word · {TEACH_TIMES} times"));
                        return Nav::Stay;
                    }
                    Row::User(i) => {
                        if let Some(u) = self.prefs.user.get_mut(i) {
                            u.action = u.action.step(by);
                        } else {
                            *cx.say = Some("enter teaches it first".into());
                            return Nav::Stay;
                        }
                    }
                }
                self.save(cx);
            }
            Key::Char('d') => {
                if let Row::User(i) = row {
                    if i < self.prefs.user.len() {
                        self.prefs.user.remove(i);
                        *cx.say = Some(format!("your word {} forgotten", i + 1));
                        self.save(cx);
                    }
                } else if let Row::Word(i) = row {
                    self.prefs.set_action(LABELS[i], default_action(LABELS[i]));
                    self.save(cx);
                }
            }
            other => crate::screens::list_key(&mut self.list, other, rows.len()),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        let width = W - 2.0 * PAD - 4.0;
        let cap_lh = ty.caption.size + 4.0;
        // What it is, in a line or two.
        let what = if self.teaching.is_some() {
            "Hold alt and say your word; let go. Esc stops.".to_owned()
        } else if !crate::smartcam::chosen() && !self.frozen {
            "Needs the smart camera on the Grove port (Settings › Camera).".to_owned()
        } else {
            "Hold alt, say a word, let go: the smart camera's NPU hears it.".to_owned()
        };
        let lines = crate::help::wrap(g, &what, ty.caption, width);
        for (i, line) in lines.iter().take(2).enumerate() {
            g.text_at(PAD + 2.0, top + 1.0 + (0.5 + i as f32) * cap_lh, Align::Left, line, ty.caption, t.dim);
        }
        let rows_top = top + 5.0 + 2.0 * cap_lh;
        let rows = self.rows();
        for (i, y) in self.list.layout(g, rows.len(), rows_top, row_h, 3.0, bottom) {
            let (name, value, hot) = self.row_text(rows[i]);
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let (name_style, value_style, name_text) = if g.theme == crate::look::Theme::Bezel { (ty.label, ty.body, name.to_lowercase()) } else { (ty.body, ty.label, name.clone()) };
            let room = r.w - 30.0 - g.measure(&name_text, name_style);
            let value = if g.measure(&value, value_style) <= room { value.clone() } else { g.text.fit(&value, &value_style, room) };
            let color = if hot { t.signal } else if value == "nothing" { t.dim } else { t.ink };
            g.row(r, &name, &value, color, self.list.focus == i);
        }
        self.list.scrollbar(g, rows.len(), rows_top, bottom);
        let hint: &[&str] = match rows.get(self.list.focus) {
            _ if self.teaching.is_some() => &["hold alt: say it · esc: stop", "alt: say it · esc: stop", "alt · esc"],
            Some(Row::Word(_)) => &["←→: its action · d: as it was", "←→: action · d: reset", "←→ · d"],
            Some(Row::User(_)) => &["enter: teach · ←→: action · d: forget", "enter: teach · d: forget", "enter · d"],
            _ => &["enter: change · esc: back", "enter: change", "enter"],
        };
        g.hint_any(hint);
    }
}

// ------------------------------------------------------------------- tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smartcam::speech::{Heard, Vad};

    fn heard(top: &[(usize, f32)], speech_ms: u32) -> Heard {
        Heard { vad: Vad { state: 2, start_ms: 100, end_ms: 100 + speech_ms, speech_ms }, top: top.to_vec(), embedding: (0..64).map(|i| (i as f32 * 0.3).sin()).collect(), ..Heard::default() }
    }

    #[test]
    fn the_labels_are_the_models() {
        let spec: serde_json::Value = serde_json::from_str(include_str!("../../../../tools/unitv/speech/kws.json")).unwrap();
        let labels: Vec<&str> = spec["labels"].as_array().unwrap().iter().map(|l| l.as_str().unwrap()).collect();
        assert_eq!(labels, LABELS);
        assert_eq!(spec["commands"].as_array().unwrap().len(), COMMANDS);
        assert!((spec["thresholds"]["command"].as_f64().unwrap() as f32 - THRESHOLD).abs() < 1e-6);
        assert!((spec["thresholds"]["user_word_cosine_prototype"].as_f64().unwrap() as f32 - USER_COSINE).abs() < 1e-6);
    }

    #[test]
    fn a_sure_command_acts_and_anything_else_is_dictation_or_silence() {
        let p = Prefs::default();
        assert_eq!(decide(&heard(&[(8, 0.93), (9, 0.03)], 500), &p), Decision::Command { word: "stop".into(), action: Action::StopTimers, score: 0.93 });
        assert_eq!(decide(&heard(&[(8, 0.6)], 500), &p), Decision::Speech, "not sure enough");
        assert_eq!(decide(&heard(&[(25, 0.9)], 500), &p), Decision::Speech, "house is known only to be told apart");
        assert_eq!(decide(&heard(&[(8, 0.95)], 2400), &p), Decision::Speech, "longer than a word");
        let quiet = Heard { vad: Vad::default(), top: vec![(30, 0.97)], ..Heard::default() };
        assert_eq!(decide(&quiet, &p), Decision::Silence);
        // One to six are Team's calls; a chosen action replaces the default.
        assert_eq!(p.action("three"), Action::TeamCall(3));
        let mut q = p.clone();
        q.set_action("three", Action::Timer(10));
        assert_eq!(decide(&heard(&[(13, 0.9)], 400), &q), Decision::Command { word: "three".into(), action: Action::Timer(10), score: 0.9 });
        q.set_action("three", Action::TeamCall(3));
        assert!(q.actions.is_empty(), "back to the default: nothing kept");
    }

    #[test]
    fn a_taught_word_is_checked_first_by_its_average() {
        let said: Vec<Vec<f32>> = (0..3).map(|k| (0..64).map(|i| ((i as f32) * 0.3).sin() + 0.05 * (k as f32 + i as f32).cos()).collect()).collect();
        let mut p = Prefs::default();
        p.user.push(UserWord { action: Action::Open("Camera"), mean: prototype(&said) });
        // Close to it: that word, even though the spotter was sure of "stop".
        let d = decide(&heard(&[(8, 0.93)], 500), &p);
        assert!(matches!(&d, Decision::Command { action: Action::Open("Camera"), score, .. } if *score > 0.95), "{d:?}");
        // Far from it: the built-in rule.
        let far = Heard { embedding: (0..64).map(|i| ((i as f32) * 1.7).cos()).collect(), ..heard(&[(8, 0.93)], 500) };
        assert!(matches!(decide(&far, &p), Decision::Command { action: Action::StopTimers, .. }));
        assert!((cosine(&prototype(&said), &unit(&said[0])) - 1.0).abs() < 0.02);
    }

    #[test]
    fn prefs_round_trip() {
        let mut p = Prefs { ready: true, dictation: Engine::Local, ..Prefs::default() };
        p.set_action("go", Action::Open("Clock"));
        p.user.push(UserWord { action: Action::TeamCall(2), mean: vec![0.125; 64] });
        assert_eq!(Prefs::parse(&p.json()), Some(p.clone()));
        assert_eq!(Prefs::parse("{}"), Some(Prefs::default()));
        for a in CHOICES {
            assert_eq!(Action::from_key_name(&a.key_name()), Some(a), "{a:?}");
        }
        assert_eq!(Action::Nothing.step(-1), Action::Open("Clock"));
    }

    #[test]
    fn the_handhelds_vad_finds_the_voice_and_trims_to_it() {
        // 0.3 s of room, 0.5 s of voice, 1 s of room.
        let mut s: Vec<i16> = (0..4800).map(|i| ((i * 7919) % 61) as i16 - 30).collect();
        s.extend((0..8000).map(|i| ((i as f32 * 0.07).sin() * 8000.0) as i16));
        s.extend((0..16000).map(|i| ((i * 7919) % 61) as i16 - 30));
        let v = vad(&s);
        assert_eq!(v.state, 2, "ended after the quiet");
        assert!((180..320).contains(&v.start_ms), "{v:?}");
        assert!((780..860).contains(&v.end_ms), "{v:?}");
        let t = trim(&s, &v);
        // The voice (0.5 s), the onset's 0.1 s before it and 0.2 s after.
        assert!(t.len() > 8000 && t.len() < 14_500, "{}", t.len());
        assert!(trim(&s[..4800], &vad(&s[..4800])).is_empty(), "nothing but the room");
    }
}
