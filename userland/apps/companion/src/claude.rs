//! Claude on the handheld: Catalyst Companion as the bridge between the
//! handheld's Claude app and Claude Code on this computer
//! (`catalyst-claude-bridge`).
//!
//!     catalyst-companion --claude-bridge      the bridge, in the tray (one at a time;
//!                                             a second one shows the first's window)
//!     catalyst-companion --claude-approver    its permission tool (Claude Code runs it)
//!     catalyst-companion --claude-shot DIR    its window, as PNGs (made-up computer)
//!
//! The window is one **Set up**: it does every step by itself
//! (`catalyst_claude_bridge::setup`) and stops only where the person signs
//! in or approves something in the browser; each step says how it stands.
//! Then the QR code and the six-digit code. Keeping the laptop awake on
//! mains power is said plainly first, as an option ticked by default, and
//! its old values can be put back. Closing the window leaves the bridge in
//! the tray, which looks again at each start and repairs what drifted.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use bezel::{Align, Canvas, Color, Icons, Rect, Text, TextStyle, Tokens};
use catalyst_claude_bridge::setup::{self, Progress, Saved, Shared, State, Step};
use catalyst_claude_bridge::{system, Bridge, Config, Invite, PairState};

use crate::ui::{Kind, Ui, CRIMSON, GOOD, K};

pub const W: usize = 760;
pub const H: usize = 680;
const X0: f32 = 40.0;
const CW: f32 = W as f32 - 2.0 * X0;
const AMBER: Color = Color::hex(0xf2b84b);

fn tokens() -> Tokens {
    Tokens::of(bezel::Accent::Rose, bezel::Tone::Dark)
}

/// The bridge's folder: `%APPDATA%\Catalyst Companion\claude`.
pub fn state_dir() -> PathBuf {
    crate::win::app_dir().join("claude")
}

pub fn approver_main() -> i32 {
    catalyst_claude_bridge::approve::main()
}

/// What Start with Windows runs.
fn at_login_command() -> String {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("catalyst-companion.exe"));
    format!("\"{}\" --claude-bridge", exe.display())
}

fn real() -> system::Real {
    system::Real::new(at_login_command())
}

// ---------------------------------------------------------------- state ---

/// A pairing on show: its QR code's modules, the code, when it began.
pub struct Showing {
    pub invite: Invite,
    pub code: String,
    pub modules: Vec<Vec<bool>>,
    pub began: Instant,
}

/// The QR code for an invite: all but the name in alphanumeric mode, the
/// name as bytes, low error correction (a screen is clean), so it stays a
/// small code the handheld's 320 × 240 preview reads.
pub fn qr_modules(invite: &Invite) -> Vec<Vec<bool>> {
    use qrcodegen::{QrCode, QrCodeEcc, QrSegment};
    let (head, name) = invite.qr_parts();
    let segs = vec![QrSegment::make_alphanumeric(&head), QrSegment::make_bytes(name.as_bytes())];
    let Ok(qr) = QrCode::encode_segments(&segs, QrCodeEcc::Low) else { return Vec::new() };
    let n = qr.size();
    (0..n).map(|y| (0..n).map(|x| qr.get_module(x, y)).collect()).collect()
}

/// What the setup, the tray and the window share across the window's
/// openings: the progress, whether a setup is running, the code on show,
/// what the last start repaired, and a word for the window.
struct Live {
    shared: Arc<Shared>,
    running: AtomicBool,
    showing: Mutex<Option<Showing>>,
    notices: Mutex<Vec<String>>,
    toast: Mutex<Option<(String, bool)>>,
}

fn live() -> &'static Live {
    static LIVE: OnceLock<Live> = OnceLock::new();
    LIVE.get_or_init(|| Live { shared: Arc::new(Shared::default()), running: AtomicBool::new(false), showing: Mutex::new(None), notices: Mutex::new(Vec::new()), toast: Mutex::new(None) })
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// What the window asks to be done (it only draws; the loop does these).
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    SetUp,
    PairAgain,
    HideCode,
    Unpair,
    Restore,
    KeepAwake(bool),
    AtLogin(bool),
    Paste(String),
    Open(String),
}

pub struct Window {
    pub progress: Progress,
    pub pair: Option<PairState>,
    pub busy: (usize, usize),
    pub showing: Option<Showing>,
    pub keep_awake: bool,
    pub at_login: bool,
    /// There are power values kept to put back.
    pub power_saved: bool,
    /// Just unpaired, with power values to put back: offered.
    pub offer_restore: bool,
    /// What the last start of the bridge repaired or found.
    pub notices: Vec<String>,
    pub running: bool,
    pub looking: bool,
    pub code: String,
    pub toast: Option<(String, bool, Instant)>,
    confirm_unpair: bool,
}

impl Window {
    pub fn new(progress: Progress) -> Window {
        Window { progress, pair: None, busy: (0, 0), showing: None, keep_awake: true, at_login: false, power_saved: false, offer_restore: false, notices: Vec::new(), running: false, looking: false, code: String::new(), toast: None, confirm_unpair: false }
    }

    fn paired(&self) -> Option<String> {
        self.pair.as_ref().and_then(|p| p.paired.clone())
    }

    /// The pairing step, from the bridge rather than the setup.
    fn pair_row(&self) -> (State, String) {
        match (self.paired(), &self.showing, self.progress.ready()) {
            (_, Some(_), _) => (State::Working, "scan the code below with the handheld's Camera".into()),
            (Some(h), None, _) => (State::Done, match self.busy {
                (_, a) if a > 0 => format!("paired with {h} · {a} waiting for an answer there"),
                (t, _) if t > 0 => format!("paired with {h} · Claude working in {t}"),
                _ => format!("paired with {h}"),
            }),
            (None, None, true) => (State::Pending, "Set up shows the code".into()),
            (None, None, false) => (State::Pending, "after the steps above".into()),
        }
    }

    /// One frame: drawn, and what was clicked asked for.
    pub fn frame(&mut self, ui: &mut Ui) -> Vec<Act> {
        let mut acts = Vec::new();
        let t = ui.t;
        crate::ui::ground(ui.canvas, &t);
        crate::ui::bolt(ui.canvas, X0 + 14.0, 46.0, 30.0);
        let nw = ui.put(X0 + 36.0, 46.0, "Claude", TextStyle::sans(24.0, 700.0), t.ink, Align::Left);
        ui.put(X0 + 42.0 + nw, 46.0, "on the handheld", TextStyle::sans(24.0, 300.0), t.dim, Align::Left);
        ui.put_fit(X0, 78.0, "Your Claude Code sessions here, worked from Catalyst. Only text crosses, sealed end to end.", TextStyle::body(14.0), t.dim, CW);
        let mut y = 98.0;
        // What the last start put right, or found needing you.
        if !self.notices.is_empty() {
            // Here, the tray's "open the window" goes without saying.
            let said: Vec<&str> = self.notices.iter().map(|n| n.split(": open the window").next().unwrap_or(n)).collect();
            let words = format!("When the bridge started: {}", said.join(" · "));
            ui.icon("sync", X0 + 8.0, y + 10.0, 15.0, AMBER);
            y = ui.para(X0 + 22.0, y, CW - 22.0, &words, TextStyle::body(13.0), t.dim, 19.0) + 4.0;
        }
        // The steps.
        let rows: Vec<setup::Row> = self.progress.rows.clone();
        // A row grows a line for a failure's fix, a page or a code, and
        // (kept awake) the way back to the power settings as they were.
        let second = |r: &setup::Row| (r.state == State::Failed && !r.fix.is_empty()) || (r.state == State::Waiting && (r.url.is_some() || r.wants_code)) || (r.step == Step::Power && self.power_saved && r.state == State::Done);
        let heights: Vec<f32> = rows.iter().map(|r| 44.0 + if second(r) { 26.0 } else { 0.0 }).collect();
        let card = Rect::new(X0, y + 4.0, CW, heights.iter().sum::<f32>() + 12.0);
        ui.card(card);
        let mut ry = card.y + 6.0;
        for (i, r) in rows.iter().enumerate() {
            let (state, words) = match r.step {
                Step::Pair => self.pair_row(),
                // Unticked below: said at once, before the next look.
                Step::Power if !self.keep_awake && !self.running && !matches!(r.state, State::Done | State::Failed) => (State::Skipped, "left as it is".to_owned()),
                _ => (r.state, r.words.clone()),
            };
            let cy = ry + 22.0;
            let icon = match r.step {
                Step::Claude => "terminal",
                Step::Tailscale => "lan",
                Step::Bridge => "hub",
                Step::Power => "power_settings_new",
                Step::Pair => "qr_code_2",
            };
            ui.icon(icon, card.x + 26.0, cy, 18.0, t.dim);
            ui.put(card.x + 46.0, cy, r.step.name(), TextStyle::sans(15.0, 600.0), t.ink, Align::Left);
            // How it stands.
            let mx = card.x + 226.0;
            // The bridge's row carries Start with Windows at its end.
            let end = if r.step == Step::Bridge {
                let label = "start with Windows";
                let lw = ui.measure(label, TextStyle::body(13.0));
                ui.put(card.right() - 72.0 - lw, cy, label, TextStyle::body(13.0), t.dim, Align::Left);
                if ui.toggle(card.right() - 62.0, cy, self.at_login) {
                    acts.push(Act::AtLogin(!self.at_login));
                }
                card.right() - 84.0 - lw
            } else {
                card.right() - 16.0
            };
            match state {
                State::Done => ui.icon("check_circle", mx, cy, 17.0, GOOD),
                State::Failed => ui.icon("error", mx, cy, 17.0, CRIMSON),
                State::Waiting => ui.icon("front_hand", mx, cy, 17.0, AMBER),
                State::Working => {
                    let spin = ui.time * 5.0;
                    ui.canvas.arc(mx, cy, 7.0, 2.2, spin, 4.2, CRIMSON, 1.0);
                }
                State::Skipped => ui.canvas.fill_rect(Rect::new(mx - 6.0, cy - 1.0, 12.0, 2.0), t.faint, 1.0),
                State::Pending => ui.canvas.circle(mx, cy, 3.5, t.faint, 1.0),
            }
            let shown = match (words.is_empty(), self.looking) {
                (true, true) => "looking…".to_owned(),
                _ => words.clone(),
            };
            let color = match state {
                State::Done | State::Working => t.ink,
                State::Waiting => AMBER,
                State::Failed => CRIMSON.mix(Color::WHITE, 0.25),
                _ => t.dim,
            };
            ui.put_fit(mx + 16.0, cy, &shown, TextStyle::body(14.0), color, end - mx - 16.0);
            // A failure's fix; a page to open again; a code to paste; the
            // power settings put back.
            let sy = cy + 26.0;
            if state == State::Failed && !r.fix.is_empty() {
                ui.put_fit(mx + 16.0, sy, &format!("Fix: {}", r.fix), TextStyle::body(13.0), t.dim, card.right() - mx - 32.0);
            } else if state == State::Waiting && r.wants_code {
                let field = Rect::new(mx + 16.0, sy - 14.0, 300.0, 28.0);
                ui.field("signin-code", field, "the code the browser shows", &mut self.code, false);
                if ui.button(Rect::new(field.right() + 8.0, sy - 14.0, 72.0, 28.0), "Send", Kind::Secondary, !self.code.trim().is_empty()) {
                    acts.push(Act::Paste(std::mem::take(&mut self.code)));
                }
            } else if state == State::Waiting {
                if let Some(url) = &r.url {
                    let label = "Open the page again";
                    let lw = ui.button_w(label);
                    if ui.button(Rect::new(mx + 16.0, sy - 14.0, lw, 28.0), label, Kind::Quiet, true) {
                        acts.push(Act::Open(url.clone()));
                    }
                }
            } else if r.step == Step::Power && self.power_saved && state == State::Done {
                let label = "Restore my power settings";
                let lw = ui.button_w(label);
                if ui.button(Rect::new(mx + 16.0, sy - 14.0, lw, 28.0), label, Kind::Quiet, !self.running) {
                    acts.push(Act::Restore);
                }
            }
            ry += heights[i];
            if i + 1 < rows.len() {
                ui.canvas.fill_rect(Rect::new(card.x + 20.0, ry - 1.0, card.w - 40.0, 1.0), t.ink, 0.06);
            }
        }
        let below = card.bottom() + 16.0;
        let foot = H as f32 - 60.0;
        if let Some(s) = &self.showing {
            Window::draw_pairing(ui, s, below, foot);
        } else {
            self.draw_below(ui, below, &mut acts);
        }
        // The buttons.
        let paired = self.paired().is_some();
        let (label, enabled) = if self.running {
            ("Setting up…", false)
        } else if self.showing.is_some() {
            ("Hide the code", true)
        } else if self.progress.ready() && paired {
            ("Check again", true)
        } else {
            ("Set up", !self.looking)
        };
        let mut items: Vec<(&str, Kind, bool)> = vec![(label, Kind::Primary, enabled)];
        if paired && self.showing.is_none() {
            items.push(("Pair again", Kind::Secondary, !self.running));
        }
        let unpair = if self.confirm_unpair { "Click again to unpair" } else { "Unpair" };
        if paired {
            items.push((unpair, Kind::Danger, true));
        }
        let clicked = ui.buttons(X0, foot, &items).map(|i| items[i].0);
        match clicked {
            Some("Set up") | Some("Check again") => acts.push(Act::SetUp),
            Some("Hide the code") => acts.push(Act::HideCode),
            Some("Pair again") => acts.push(Act::PairAgain),
            Some("Click again to unpair") => {
                self.confirm_unpair = false;
                acts.push(Act::Unpair);
            }
            Some("Unpair") => self.confirm_unpair = true,
            _ if ui.clicked => self.confirm_unpair = false,
            _ => {}
        }
        if ui.keys.contains(&K::Escape) && self.showing.is_some() {
            acts.push(Act::HideCode);
        }
        self.draw_toast(ui);
        acts
    }

    /// Under the steps: before the power step is done, what keeping awake
    /// changes, as a ticked option; after unpairing, putting it back.
    fn draw_below(&mut self, ui: &mut Ui, top: f32, acts: &mut Vec<Act>) {
        let t = ui.t;
        if self.offer_restore && self.power_saved {
            ui.put(X0, top + 10.0, "Unpaired. This laptop is still kept awake when plugged in.", TextStyle::sans(15.0, 600.0), t.ink, Align::Left);
            ui.para(X0, top + 24.0, CW, "Restore my power settings puts sleep, hibernate and the lid back as they were before the setup.", TextStyle::body(14.0), t.dim, 20.0);
            return;
        }
        let power = self.progress.row(Step::Power).state;
        if matches!(power, State::Done) || self.progress.ready() {
            if self.paired().is_some() {
                ui.para(X0, top, CW, "The handheld reaches Claude Code here: Claude, on its Home. You can close this window; the bridge stays in the tray and checks everything again each time it starts.", TextStyle::body(14.0), t.dim, 21.0);
            }
            return;
        }
        // The option, said plainly.
        let r = Rect::new(X0, top, 22.0, 22.0);
        let hit = Rect::new(X0 - 4.0, top - 4.0, CW, 30.0);
        if !self.running && ui.hit(hit) {
            acts.push(Act::KeepAwake(!self.keep_awake));
        }
        if self.keep_awake {
            ui.canvas.rrect(r, 6.0, CRIMSON, 1.0);
            ui.canvas.polygon(&[(r.x + 5.0, r.y + 11.5), (r.x + 9.5, r.y + 16.0), (r.x + 17.5, r.y + 7.0), (r.x + 16.0, r.y + 5.5), (r.x + 9.5, r.y + 13.0), (r.x + 6.5, r.y + 10.0)], Color::WHITE, 1.0);
        } else {
            ui.canvas.stroke_rrect(r, 6.0, 1.5, t.dim, 1.0);
        }
        ui.put(X0 + 32.0, top + 11.0, "Keep this laptop awake when plugged in", TextStyle::sans(15.0, 600.0), t.ink, Align::Left);
        ui.para(
            X0 + 32.0,
            top + 26.0,
            CW - 32.0,
            "On mains power only: sleep after Never, hibernate after Never, closing the lid Does nothing, so Claude stays reachable while you are out. Battery settings are not touched. The values now are kept, and Restore my power settings puts them back.",
            TextStyle::body(13.0),
            t.dim,
            19.0,
        );
    }

    fn draw_pairing(ui: &mut Ui, s: &Showing, top: f32, foot: f32) {
        let t = ui.t;
        let room = foot - top - 14.0;
        // The code, dark on white with its quiet zone.
        let n = s.modules.len().max(1) as f32;
        let tile = room.min(300.0);
        let unit = ((tile - 16.0) / (n + 8.0)).floor().max(1.0);
        let side = unit * (n + 8.0);
        let qr = Rect::new(X0, top, side, side);
        ui.canvas.rrect(qr, 12.0, Color::WHITE, 1.0);
        for (y, row) in s.modules.iter().enumerate() {
            for (x, &on) in row.iter().enumerate() {
                if on {
                    ui.canvas.fill_rect(Rect::new(qr.x + (x as f32 + 4.0) * unit, qr.y + (y as f32 + 4.0) * unit, unit, unit), Color::hex(0x111111), 1.0);
                }
            }
        }
        let x = qr.right() + 32.0;
        let w = W as f32 - X0 - x;
        ui.put(x, top + 12.0, "Scan it with the handheld", TextStyle::sans(18.0, 650.0), t.ink, Align::Left);
        let y = ui.para(x, top + 26.0, w, "Camera, in scan mode. Or in Claude, choose Type its address and code:", TextStyle::body(14.0), t.dim, 20.0);
        let address = s.invite.address.trim_start_matches("https://");
        ui.put_fit(x, y + 16.0, address, TextStyle::sans(15.0, 600.0), t.ink, w);
        let code = format!("{} {}", &s.code[..3], &s.code[3..]);
        ui.put(x, y + 56.0, &code, TextStyle::display(38.0), t.ink, Align::Left);
        let left = (catalyst_companion_proto::CODE_LIFETIME as f32 - s.began.elapsed().as_secs_f32()).max(0.0) as u64;
        let words = if left > 0 { format!("the code works for {}:{:02} more, five tries", left / 60, left % 60) } else { "the code has run out: the QR code still works".into() };
        ui.put_fit(x, y + 90.0, &words, TextStyle::body(13.0), t.faint, w);
    }

    fn draw_toast(&mut self, ui: &mut Ui) {
        let Some((text, error, at)) = self.toast.clone() else { return };
        let age = at.elapsed().as_secs_f32();
        let life = if error { 7.0 } else { 4.5 };
        if age > life {
            self.toast = None;
            return;
        }
        let alpha = (age * 6.0).min(1.0).min((life - age) * 3.0).clamp(0.0, 1.0);
        let style = TextStyle::sans(14.0, 550.0);
        let text = ui.text.fit(&text, &style, W as f32 - 120.0);
        let w = ui.measure(&text, style) + 48.0;
        let r = Rect::new(W as f32 * 0.5 - w * 0.5, H as f32 - 108.0 + (1.0 - alpha) * 8.0, w, 40.0);
        ui.canvas.rrect(r.offset(0.0, 3.0), 20.0, Color::BLACK, 0.3 * alpha);
        ui.canvas.rrect(r, 20.0, ui.t.surface3, alpha);
        ui.dot(r.x + 18.0, r.cy(), if error { CRIMSON } else { GOOD });
        ui.put_alpha(r.x + 30.0, r.cy(), &text, style, ui.t.ink, Align::Left, alpha);
    }
}

// --------------------------------------------------------------- acting ---

fn options(bridge: Option<&Bridge>, error: &Option<String>) -> setup::Options {
    setup::Options::new(match (bridge, error) {
        (Some(_), _) => Ok(()),
        (None, Some(e)) => Err(e.clone()),
        (None, None) => Err("the bridge is not running".into()),
    })
}

/// Show a new pairing code for `address`.
fn begin_pairing(bridge: &Bridge, address: &str) {
    match bridge.begin_pairing(address) {
        Ok((invite, code)) => {
            let modules = qr_modules(&invite);
            *lock(&live().showing) = Some(Showing { invite, code, modules, began: Instant::now() });
        }
        Err(why) => *lock(&live().toast) = Some((why, true)),
    }
}

/// The setup, on a thread of its own; the code shown at its end when no
/// handheld is paired yet (or `pair` asks for it).
fn start_setup(bridge: Option<Bridge>, error: Option<String>, pair: bool) {
    let l = live();
    if l.running.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let opts = options(bridge.as_ref(), &error);
        let address = setup::run(&real(), &state_dir(), &opts, &l.shared);
        if let (Some(b), Some(a)) = (&bridge, address) {
            if pair || b.pair_state().paired.is_none() {
                begin_pairing(b, &a);
            }
        }
        l.running.store(false, Ordering::SeqCst);
    });
}

/// How things stand, looked at only, when the window opens.
fn start_look(bridge: Option<Bridge>, error: Option<String>) {
    let l = live();
    if l.running.load(Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let p = setup::look(&real(), &Saved::load(&state_dir()), &options(bridge.as_ref(), &error));
        if !l.running.load(Ordering::SeqCst) {
            *lock(&l.shared.progress) = p;
        }
    });
}

fn act(a: Act, w: &mut Window, bridge: Option<&Bridge>, error: &Option<String>) {
    let dir = state_dir();
    match a {
        Act::SetUp => {
            w.offer_restore = false;
            start_setup(bridge.cloned(), error.clone(), false);
        }
        Act::PairAgain => start_setup(bridge.cloned(), error.clone(), true),
        Act::HideCode => {
            *lock(&live().showing) = None;
            if let Some(b) = bridge {
                b.end_pairing();
            }
        }
        Act::Unpair => {
            if let Some(b) = bridge {
                b.unpair();
            }
            w.offer_restore = true;
            w.toast = Some(("unpaired: the handheld's key opens nothing now".into(), false, Instant::now()));
        }
        Act::Restore => {
            std::thread::spawn(move || {
                let r = setup::restore_power(&real(), &state_dir());
                *lock(&live().toast) = Some(match r {
                    Ok(words) => (words, false),
                    Err(why) => (format!("not restored: {why}"), true),
                });
            });
            w.offer_restore = false;
        }
        Act::KeepAwake(on) => {
            let mut s = Saved::load(&dir);
            s.keep_awake = on;
            s.save(&dir);
            w.keep_awake = on;
        }
        Act::AtLogin(on) => match system::set_at_login(on, &at_login_command()) {
            Ok(()) => {
                let mut s = Saved::load(&dir);
                s.at_login_declined = !on;
                s.save(&dir);
                w.at_login = on;
            }
            Err(why) => w.toast = Some((why, true, Instant::now())),
        },
        Act::Paste(code) => *lock(&live().shared.pasted) = Some(code),
        Act::Open(url) => {
            use catalyst_claude_bridge::system::System;
            real().open_url(&url);
        }
    }
}

// --------------------------------------------------------------- bridge ---

fn tip(bridge: &Bridge) -> String {
    let (turns, asks) = bridge.busy();
    let who = bridge.pair_state().paired.map_or("not paired".to_owned(), |h| format!("paired with {h}"));
    let notice = lock(&live().notices).first().cloned().map(|n| format!(" · {n}")).unwrap_or_default();
    match (turns, asks) {
        (_, a) if a > 0 => format!("Claude bridge · {who} · {a} waiting for the handheld"),
        (t, _) if t > 0 => format!("Claude bridge · {who} · {t} working"),
        _ => format!("Claude bridge · {who}{notice}"),
    }
}

/// `--claude-bridge`: the bridge in the tray, and its window when asked
/// (at once with `show`, or the first time, before anything is set up).
#[cfg(windows)]
pub fn bridge_main(show: bool) -> i32 {
    use crate::tray::{Ask, Tray};
    if crate::tray::show_running() {
        return 0;
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("catalyst-companion.exe"));
    // Claude Code may not be installed yet: the bridge starts all the same,
    // and Set up installs it (the claude it runs is found again each turn).
    let claude = catalyst_claude_bridge::runner::find_claude().unwrap_or_else(|| PathBuf::from("claude.exe"));
    let config = Config { claude, ..Config::standard(state_dir(), (exe, vec!["--claude-approver".into()]), crate::win::computer_name()).unwrap_or_else(|_| fallback_config()) };
    let (bridge, error) = match Bridge::start(config) {
        Ok(b) => (Some(b), None),
        Err(why) => (None, Some(why)),
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let set_up = Saved::load(&state_dir()).completed;
    if show || !set_up || bridge.as_ref().is_none_or(|b| b.pair_state().paired.is_none()) {
        let _ = tx.send(Ask::Show);
    }
    let tray: Arc<Mutex<Option<Tray>>> = Arc::new(Mutex::new(None));
    let tray_now = |tray: &Arc<Mutex<Option<Tray>>>| *tray.lock().unwrap_or_else(|e| e.into_inner());
    // A handheld paired, or unpaired: the tooltip says so.
    if let Some(b) = &bridge {
        let (t, b2) = (tray.clone(), b.clone());
        b.on_change(move || {
            if let Some(t) = tray_now(&t) {
                t.tip(&tip(&b2));
            }
        });
    }
    // Looked at again at each start: what drifted repaired, and said.
    {
        let (t, b, error) = (tray.clone(), bridge.clone(), error.clone());
        std::thread::spawn(move || {
            let said = setup::repair(&real(), &state_dir(), &options(b.as_ref(), &error));
            if said.is_empty() {
                return;
            }
            *lock(&live().notices) = said.clone();
            for _ in 0..50 {
                if let Some(t) = tray_now(&t) {
                    t.notify("Claude bridge", &said.join("\n"));
                    if let Some(b) = &b {
                        t.tip(&tip(b));
                    }
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        });
    }
    // What the tray asks, on a thread of its own; the tray on this one.
    let worker = {
        let (tray, bridge) = (tray.clone(), bridge.clone());
        std::thread::spawn(move || {
            let open = Arc::new(AtomicBool::new(false));
            while let Ok(ask) = rx.recv() {
                match ask {
                    Ask::Show if !open.swap(true, Ordering::Relaxed) => {
                        let (b, open, error, tray) = (bridge.clone(), open.clone(), error.clone(), tray.clone());
                        std::thread::spawn(move || {
                            window(b.as_ref(), error);
                            open.store(false, Ordering::Relaxed);
                            if let (Some(b), Some(t)) = (&b, tray_now(&tray)) {
                                t.tip(&tip(b));
                            }
                        });
                    }
                    Ask::Show => {}
                    Ask::Quit => {
                        if let Some(b) = &bridge {
                            b.stop();
                        }
                        if let Some(t) = tray_now(&tray) {
                            t.quit();
                        }
                        break;
                    }
                }
            }
        })
    };
    let start_tip = bridge.as_ref().map_or_else(|| "Claude bridge · not running".to_owned(), tip);
    let ticking = bridge.clone();
    crate::tray::run(tx, &start_tip, move |t| {
        *tray.lock().unwrap_or_else(|e| e.into_inner()) = Some(t);
        // The tooltip's count of turns and requests, from the bridge's own
        // state (nothing on the network) every 20 s.
        if let Some(b) = ticking {
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(20));
                t.tip(&tip(&b));
            });
        }
    });
    let _ = worker.join();
    0
}

/// The bridge's settings when Claude Code is not found yet.
#[cfg(windows)]
fn fallback_config() -> Config {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("catalyst-companion.exe"));
    Config {
        state_dir: state_dir(),
        projects_dir: catalyst_claude_bridge::sessions::projects_dir(),
        claude: PathBuf::from("claude.exe"),
        approver: (exe, vec!["--claude-approver".into()]),
        port: catalyst_claude_bridge::PORT,
        approver_port: catalyst_claude_bridge::APPROVER_PORT,
        computer: crate::win::computer_name(),
        approval_seconds: catalyst_claude_bridge::APPROVAL_SECONDS,
        protect: true,
    }
}

#[cfg(not(windows))]
pub fn bridge_main(_show: bool) -> i32 {
    eprintln!("the Claude bridge runs on Windows");
    1
}

/// The window, until it is closed (the bridge, and a setup under way, carry on).
#[cfg(not(target_arch = "aarch64"))]
fn window(bridge: Option<&Bridge>, error: Option<String>) {
    use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Window as Win, WindowOptions};
    let Ok(mut win) = Win::new("Claude on the handheld · Catalyst Companion", W, H, WindowOptions::default()) else { return };
    win.set_target_fps(30);
    let l = live();
    let mut state = Window::new(l.shared.snapshot());
    state.looking = !l.running.load(Ordering::SeqCst);
    start_look(bridge.cloned(), error.clone());
    let typed = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    win.set_input_callback(Box::new(crate::Typed(typed.clone())));
    let (mut canvas, mut text, mut icons) = (Canvas::new(W as i32, H as i32), Text::new(), Icons::new());
    let mut focus: Option<String> = None;
    let mut was_down = false;
    let started = Instant::now();
    let mut slow = Instant::now() - std::time::Duration::from_secs(10);
    while win.is_open() {
        // What the setup and the bridge say, each frame; the files and the
        // Run key once a second.
        state.progress = l.shared.snapshot();
        state.running = l.running.load(Ordering::SeqCst);
        if state.looking && state.progress.rows.iter().any(|r| !r.words.is_empty()) {
            state.looking = false;
        }
        state.pair = bridge.map(Bridge::pair_state);
        state.busy = bridge.map_or((0, 0), Bridge::busy);
        state.notices = lock(&l.notices).clone();
        if let Some((words, error)) = lock(&l.toast).take() {
            state.toast = Some((words, error, Instant::now()));
        }
        // Paired meanwhile: the code goes, and it says so.
        {
            let mut showing = lock(&l.showing);
            if showing.is_some() && state.pair.as_ref().is_some_and(|p| p.offering.is_none()) {
                *showing = None;
                if let Some(h) = state.pair.as_ref().and_then(|p| p.paired.clone()) {
                    state.toast = Some((format!("paired with {h}"), false, Instant::now()));
                }
            }
            state.showing = showing.as_ref().map(|s| Showing { invite: s.invite.clone(), code: s.code.clone(), modules: s.modules.clone(), began: s.began });
        }
        if slow.elapsed().as_secs() >= 1 {
            let saved = Saved::load(&state_dir());
            state.keep_awake = saved.keep_awake;
            state.power_saved = saved.power_before.is_some();
            state.at_login = system::at_login();
            slow = Instant::now();
        }
        let mut keys = Vec::new();
        for key in win.get_keys_pressed(KeyRepeat::Yes) {
            keys.push(match key {
                Key::Escape => K::Escape,
                Key::Enter | Key::NumPadEnter => K::Enter,
                Key::Backspace => K::Backspace,
                _ => continue,
            });
        }
        let down = win.get_mouse_down(MouseButton::Left);
        let clicked = down && !was_down;
        was_down = down;
        if clicked {
            focus = None;
        }
        let mouse = win.get_mouse_pos(MouseMode::Pass).unwrap_or((-1.0, -1.0));
        let chars: Vec<char> = typed.borrow_mut().drain(..).filter(|c| !c.is_control()).collect();
        let mut ui = Ui { canvas: &mut canvas, text: &mut text, icons: &mut icons, t: tokens(), mouse, clicked, wheel: 0.0, typed: chars, keys, focus: &mut focus, fields: Vec::new(), pointing: false, time: started.elapsed().as_secs_f32(), pasted: None };
        for a in state.frame(&mut ui) {
            act(a, &mut state, bridge, &error);
        }
        win.update_with_buffer(&canvas.px, W, H).ok();
    }
    if lock(&l.showing).take().is_some() {
        if let Some(b) = bridge {
            b.end_pairing();
        }
    }
}

#[cfg(target_arch = "aarch64")]
fn window(_: Option<&Bridge>, _: Option<String>) {}

// ---------------------------------------------------------------- shots ---

/// `--claude-shot DIR`: the window in each state, as PNGs, for a made-up
/// computer.
pub fn shots(dir: PathBuf) {
    use catalyst_claude_bridge::Key;
    std::fs::create_dir_all(&dir).expect("the folder");
    let (mut canvas, mut text, mut icons) = (Canvas::new(W as i32, H as i32), Text::new(), Icons::new());
    let mut focus = None;
    let mut draw = |w: &mut Window, name: &str| {
        let mut ui = Ui { canvas: &mut canvas, text: &mut text, icons: &mut icons, t: tokens(), mouse: (-1.0, -1.0), clicked: false, wheel: 0.0, typed: Vec::new(), keys: Vec::new(), focus: &mut focus, fields: Vec::new(), pointing: false, time: 0.2, pasted: None };
        w.frame(&mut ui);
        crate::save_png_size(&canvas, &dir.join(format!("{name}.png")), W, H);
    };
    let set = |p: &mut Progress, step: Step, state: State, words: &str| {
        let r = p.rows.iter_mut().find(|r| r.step == step).unwrap();
        r.state = state;
        r.words = words.into();
    };
    let unpaired = Some(PairState { paired: None, offering: None });
    // A fresh laptop: nothing yet.
    let mut p = Progress::default();
    set(&mut p, Step::Claude, State::Pending, "not installed");
    set(&mut p, Step::Tailscale, State::Pending, "not installed");
    set(&mut p, Step::Bridge, State::Pending, "running");
    set(&mut p, Step::Power, State::Pending, "would change on mains power: sleep, hibernate, the lid");
    let mut w = Window::new(p.clone());
    w.pair = unpaired.clone();
    draw(&mut w, "claude-1-fresh");
    // Under way: Claude Code done, Tailscale's sign-in waiting.
    set(&mut p, Step::Claude, State::Done, "Claude Code 2.1.325 · signed in");
    set(&mut p, Step::Tailscale, State::Waiting, "sign in to Tailscale in your browser");
    p.rows[1].url = Some("https://login.tailscale.com/a/abc123".into());
    p.running = true;
    w.progress = p.clone();
    w.running = true;
    draw(&mut w, "claude-2-waiting-sign-in");
    // Claude's sign-in asking for a code.
    let mut q = p.clone();
    set(&mut q, Step::Claude, State::Waiting, "sign in to Claude in your browser");
    q.rows[0].wants_code = true;
    set(&mut q, Step::Tailscale, State::Pending, "not installed");
    w.progress = q;
    draw(&mut w, "claude-3-paste-code");
    // A sign-in that never came; Windows' permission refused.
    let mut f = p.clone();
    f.running = false;
    set(&mut f, Step::Claude, State::Failed, "no sign-in came within ten minutes");
    f.rows[0].fix = "Set up again when you can sign in".into();
    set(&mut f, Step::Tailscale, State::Done, "Funnel publishes laptop.tail1234.ts.net:10000");
    set(&mut f, Step::Bridge, State::Done, "running · starts with Windows");
    set(&mut f, Step::Power, State::Failed, "not changed: the permission was not given");
    f.rows[3].fix = "Set up again and choose Yes, or untick Keep awake".into();
    w.progress = f;
    w.at_login = true;
    w.running = false;
    draw(&mut w, "claude-4-failed");
    // All done: the code.
    let mut d = Progress::default();
    set(&mut d, Step::Claude, State::Done, "Claude Code 2.1.325 · signed in");
    set(&mut d, Step::Tailscale, State::Done, "Funnel publishes laptop.tail1234.ts.net:10000");
    set(&mut d, Step::Bridge, State::Done, "running · starts with Windows");
    set(&mut d, Step::Power, State::Done, "on mains power: never sleeps, lid does nothing");
    w.progress = d.clone();
    w.power_saved = true;
    w.at_login = true;
    let invite = Invite { address: "https://laptop.tail1234.ts.net:10000".into(), key: Key::from_bytes([0x5d; 32]), computer: "THOMAS-LAPTOP".into() };
    let modules = qr_modules(&invite);
    w.showing = Some(Showing { invite, code: "482913".into(), modules, began: Instant::now() - std::time::Duration::from_secs(48) });
    draw(&mut w, "claude-5-pair");
    // Paired.
    w.showing = None;
    w.pair = Some(PairState { paired: Some("catalyst".into()), offering: None });
    draw(&mut w, "claude-6-paired");
    // Unpaired: putting the power settings back offered.
    w.pair = unpaired.clone();
    w.offer_restore = true;
    draw(&mut w, "claude-7-unpaired-restore");
    // A later start: what it repaired, and what needs you.
    w.offer_restore = false;
    w.pair = Some(PairState { paired: Some("catalyst".into()), offering: None });
    let mut r = d.clone();
    set(&mut r, Step::Claude, State::Pending, "installed · not signed in");
    w.progress = r;
    w.notices = vec!["Funnel was off: turned it back on".into(), "Claude Code is signed out: open the window and Set up to sign in".into()];
    draw(&mut w, "claude-8-repaired");
    // Keeping awake unticked.
    let mut u = Window::new(p.clone());
    u.progress.running = false;
    set(&mut u.progress, Step::Claude, State::Pending, "not installed");
    set(&mut u.progress, Step::Tailscale, State::Pending, "not installed");
    u.keep_awake = false;
    u.pair = unpaired;
    draw(&mut u, "claude-9-keep-awake-unticked");
    println!("claude window in {}", dir.display());
}
