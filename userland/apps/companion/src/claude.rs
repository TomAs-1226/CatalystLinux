//! Claude on the handheld: Catalyst Companion as the bridge between the
//! handheld's Claude app and Claude Code on this computer
//! (`catalyst-claude-bridge`).
//!
//!     catalyst-companion --claude-bridge      the bridge, in the tray (one at a time;
//!                                             a second one shows the first's window)
//!     catalyst-companion --claude-approver    its permission tool (Claude Code runs it)
//!     catalyst-companion --claude-shot DIR    its window, as PNGs (made-up computer)
//!
//! The window says how things stand (Claude Code found, Tailscale signed
//! in, Funnel publishing the bridge, the handheld paired), the exact steps
//! for what is missing, and, to pair, the QR code and the six-digit code.
//! Closing it leaves the bridge in the tray. Nothing here changes Windows'
//! or Tailscale's settings, apart from *Start with Windows*, which the
//! person switches (a value under the user's own Run key).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bezel::{Align, Canvas, Color, Icons, Rect, Text, TextStyle, Tokens};
use catalyst_claude_bridge::{tailscale, Bridge, Config, Invite, PairState};

use crate::ui::{Kind, Ui, CRIMSON, GOOD, K};

pub const W: usize = 760;
pub const H: usize = 660;
const X0: f32 = 40.0;
const CW: f32 = W as f32 - 2.0 * X0;

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

// ---------------------------------------------------------------- state ---

/// What the window shows.
#[derive(Clone, Default)]
pub struct Looked {
    pub claude: Option<PathBuf>,
    /// Claude Code signed in on this computer (None: it could not tell).
    pub signed_in: Option<bool>,
    pub tailscale: tailscale::Tailscale,
    pub pair: Option<PairState>,
    pub busy: (usize, usize),
    pub running: bool,
    pub error: Option<String>,
}

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

pub struct Window {
    pub looked: Looked,
    pub showing: Option<Showing>,
    pub at_login: bool,
    pub toast: Option<(String, bool, Instant)>,
    confirm_unpair: bool,
}

impl Window {
    pub fn new(looked: Looked) -> Window {
        Window { looked, showing: None, at_login: at_login(), toast: None, confirm_unpair: false }
    }

    /// One frame: drawn, and what was clicked done (`act` on the bridge).
    pub fn frame(&mut self, ui: &mut Ui, bridge: Option<&Bridge>) {
        let t = ui.t;
        crate::ui::ground(ui.canvas, &t);
        crate::ui::bolt(ui.canvas, X0 + 14.0, 50.0, 30.0);
        let nw = ui.put(X0 + 36.0, 50.0, "Claude", TextStyle::sans(24.0, 700.0), t.ink, Align::Left);
        ui.put(X0 + 42.0 + nw, 50.0, "on the handheld", TextStyle::sans(24.0, 300.0), t.dim, Align::Left);
        ui.put_fit(X0, 84.0, "Your Claude Code sessions here, worked from Catalyst. Only text crosses, sealed end to end.", TextStyle::body(14.0), t.dim, CW);
        // How things stand.
        let l = self.looked.clone();
        let card = Rect::new(X0, 106.0, CW, 4.0 * 40.0 + 16.0);
        ui.card(card);
        let ts = &l.tailscale;
        let rows: [(&str, &str, String, bool); 4] = [
            ("terminal", "Claude Code", match (&l.claude, l.signed_in) {
                (None, _) => "not installed: install it, run claude once and sign in".into(),
                (Some(_), Some(false)) => "not signed in: run  claude auth login  in a terminal".into(),
                (Some(_), _) => "found, signed in".into(),
            }, l.claude.is_some() && l.signed_in != Some(false)),
            ("lan", "Tailscale", match (ts.installed, ts.running, &ts.dns_name) {
                (false, ..) => "not installed".into(),
                (true, false, _) => "not connected".into(),
                (true, true, Some(n)) => format!("connected as {n}"),
                (true, true, None) => "connected".into(),
            }, ts.running),
            ("language", "Funnel", match &ts.address {
                Some(a) => a.clone(),
                None => "not publishing the bridge yet".into(),
            }, ts.address.is_some()),
            ("smart_toy", "Handheld", match (&l.pair, l.running) {
                (_, false) => l.error.clone().unwrap_or_else(|| "the bridge is not running".into()),
                (Some(PairState { paired: Some(h), .. }), _) => match l.busy {
                    (_, asks) if asks > 0 => format!("paired with {h} · {asks} waiting for an answer there"),
                    (turns, _) if turns > 0 => format!("paired with {h} · Claude working in {turns}"),
                    _ => format!("paired with {h}"),
                },
                _ => "not paired".into(),
            }, matches!(&l.pair, Some(PairState { paired: Some(_), .. }))),
        ];
        for (i, (icon, name, value, good)) in rows.iter().enumerate() {
            let cy = card.y + 8.0 + 20.0 + i as f32 * 40.0;
            ui.icon(icon, card.x + 28.0, cy, 18.0, t.dim);
            ui.put(card.x + 48.0, cy, name, TextStyle::sans(15.0, 600.0), t.ink, Align::Left);
            ui.dot(card.x + 196.0, cy, if *good { GOOD } else { CRIMSON });
            ui.put_fit(card.x + 210.0, cy, value, TextStyle::body(14.0), if *good { t.ink } else { t.dim }, card.w - 230.0);
            if i < 3 {
                ui.canvas.fill_rect(Rect::new(card.x + 20.0, cy + 20.0, card.w - 40.0, 1.0), t.ink, 0.06);
            }
        }
        let below = card.bottom() + 18.0;
        let foot = H as f32 - 64.0;
        if let Some(s) = &self.showing {
            Window::draw_pairing(ui, s, below, foot);
        } else {
            let steps = tailscale::steps(ts, catalyst_claude_bridge::PORT);
            if !steps.is_empty() || l.claude.is_none() || l.signed_in == Some(false) {
                ui.put(X0, below + 10.0, "To reach it from the handheld", TextStyle::sans(16.0, 650.0), t.ink, Align::Left);
                let mut y = below + 26.0;
                let mut all: Vec<String> = Vec::new();
                if l.claude.is_none() {
                    all.push("Install Claude Code (claude.com/claude-code), run claude once in a terminal and sign in.".into());
                } else if l.signed_in == Some(false) {
                    all.push("Sign Claude Code in on this computer: in a terminal, run  claude auth login  (the bridge runs the claude command, which keeps its own sign-in).".into());
                }
                all.extend(steps);
                for (k, step) in all.iter().enumerate() {
                    ui.put(X0 + 6.0, y + 10.0, &format!("{}", k + 1), TextStyle::sans(14.0, 700.0), CRIMSON, Align::Left);
                    y = ui.para(X0 + 26.0, y, CW - 26.0, step, TextStyle::body(14.0), t.dim, 20.0) + 6.0;
                }
            } else {
                let words = match &l.pair {
                    Some(PairState { paired: Some(h), .. }) => format!("{h} reaches Claude Code here. On the handheld: Claude, on Home. Close this window: the bridge stays in the tray."),
                    _ => "Ready to pair. Pair a handheld shows a QR code for its Camera, and a code to type instead.".into(),
                };
                ui.para(X0, below, CW, &words, TextStyle::body(15.0), t.dim, 22.0);
                ui.para(X0, below + 54.0, CW, "Keep this computer awake while you are out: plugged in, set to never sleep and to do nothing when the lid closes (see the README).", TextStyle::body(13.0), t.faint, 19.0);
            }
        }
        // The buttons.
        let paired = matches!(&l.pair, Some(PairState { paired: Some(_), .. }));
        let can_pair = l.running && l.claude.is_some() && ts.address.is_some();
        let pair_label = if self.showing.is_some() { "Hide the code" } else if paired { "Pair again" } else { "Pair a handheld" };
        let unpair_label = if self.confirm_unpair { "Click again to unpair" } else { "Unpair" };
        let clicked = ui.buttons(X0, foot, &[(pair_label, Kind::Primary, can_pair || self.showing.is_some()), (unpair_label, Kind::Danger, paired), ("Check again", Kind::Secondary, true)]);
        // Start with Windows, at the right.
        let label = "Start with Windows";
        let lw = ui.measure(label, TextStyle::body(14.0));
        ui.put(W as f32 - X0 - 48.0 - lw, foot + 18.0, label, TextStyle::body(14.0), t.dim, Align::Left);
        if ui.toggle(W as f32 - X0 - 40.0, foot + 18.0, self.at_login) {
            match set_at_login(!self.at_login) {
                Ok(()) => self.at_login = !self.at_login,
                Err(why) => self.toast = Some((why, true, Instant::now())),
            }
        }
        let keys = ui.keys.clone();
        match clicked {
            Some(0) if self.showing.is_some() => {
                self.showing = None;
                if let Some(b) = bridge {
                    b.end_pairing();
                }
            }
            Some(0) => {
                if let (Some(b), Some(address)) = (bridge, ts.address.as_deref()) {
                    match b.begin_pairing(address) {
                        Ok((invite, code)) => {
                            let modules = qr_modules(&invite);
                            self.showing = Some(Showing { invite, code, modules, began: Instant::now() });
                        }
                        Err(why) => self.toast = Some((why, true, Instant::now())),
                    }
                }
            }
            Some(1) if self.confirm_unpair => {
                if let Some(b) = bridge {
                    b.unpair();
                }
                self.confirm_unpair = false;
                self.toast = Some(("unpaired: the handheld's key opens nothing now".into(), false, Instant::now()));
            }
            Some(1) => self.confirm_unpair = true,
            Some(2) => self.looked.tailscale = tailscale::look(catalyst_claude_bridge::PORT),
            _ if ui.clicked => self.confirm_unpair = false,
            _ => {}
        }
        if keys.contains(&K::Escape) && self.showing.is_some() {
            self.showing = None;
        }
        self.draw_toast(ui);
    }

    fn draw_pairing(ui: &mut Ui, s: &Showing, top: f32, foot: f32) {
        let t = ui.t;
        let room = foot - top - 18.0;
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
        let y = ui.para(x, top + 26.0, w, "Camera, in scan mode. Or in Claude, choose Type a code and enter this computer's address and the code:", TextStyle::body(14.0), t.dim, 20.0);
        let address = s.invite.address.trim_start_matches("https://");
        ui.put_fit(x, y + 18.0, address, TextStyle::sans(15.0, 600.0), t.ink, w);
        let code = format!("{} {}", &s.code[..3], &s.code[3..]);
        ui.put(x, y + 62.0, &code, TextStyle::display(40.0), t.ink, Align::Left);
        let left = (catalyst_companion_proto::CODE_LIFETIME as f32 - s.began.elapsed().as_secs_f32()).max(0.0) as u64;
        let words = if left > 0 { format!("the code works for {}:{:02} more, five tries", left / 60, left % 60) } else { "the code has run out: the QR code still works".into() };
        ui.put_fit(x, y + 98.0, &words, TextStyle::body(13.0), t.faint, w);
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
        let r = Rect::new(W as f32 * 0.5 - w * 0.5, H as f32 - 112.0 + (1.0 - alpha) * 8.0, w, 40.0);
        ui.canvas.rrect(r.offset(0.0, 3.0), 20.0, Color::BLACK, 0.3 * alpha);
        ui.canvas.rrect(r, 20.0, ui.t.surface3, alpha);
        ui.dot(r.x + 18.0, r.cy(), if error { CRIMSON } else { GOOD });
        ui.put_alpha(r.x + 30.0, r.cy(), &text, style, ui.t.ink, Align::Left, alpha);
    }
}

// --------------------------------------------------------- start at login ---

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "Catalyst Claude bridge";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Whether the bridge starts when this user signs in.
pub fn at_login() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
        let (key, value) = (wide(RUN_KEY), wide(RUN_VALUE));
        let mut size = 0u32;
        // SAFETY: a size query of one value under the user's own key.
        unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) == 0 }
    }
    #[cfg(not(windows))]
    false
}

/// Start the bridge when this user signs in, or stop doing so.
pub fn set_at_login(on: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Registry::{RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ};
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let command = wide(&format!("\"{}\" --claude-bridge", exe.display()));
        let (key_name, value) = (wide(RUN_KEY), wide(RUN_VALUE));
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: the user's own Run key, opened, written and closed.
        unsafe {
            if RegOpenKeyExW(HKEY_CURRENT_USER, key_name.as_ptr(), 0, KEY_SET_VALUE, &mut key) != 0 {
                return Err("Windows would not open its Run settings".into());
            }
            let r = if on { RegSetValueExW(key, value.as_ptr(), 0, REG_SZ, command.as_ptr().cast(), (command.len() * 2) as u32) } else { RegDeleteValueW(key, value.as_ptr()) };
            RegCloseKey(key);
            if r != 0 && on {
                return Err("Windows would not keep the setting".into());
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = on;
        Err("only on Windows".into())
    }
}

// --------------------------------------------------------------- bridge ---

fn look(bridge: Option<&Bridge>, error: Option<String>) -> Looked {
    Looked {
        claude: catalyst_claude_bridge::runner::find_claude(),
        signed_in: catalyst_claude_bridge::runner::find_claude().and_then(|c| catalyst_claude_bridge::runner::signed_in(&c)),
        tailscale: tailscale::look(catalyst_claude_bridge::PORT),
        pair: bridge.map(Bridge::pair_state),
        busy: bridge.map_or((0, 0), Bridge::busy),
        running: bridge.is_some(),
        error,
    }
}

fn tip(bridge: &Bridge) -> String {
    let (turns, asks) = bridge.busy();
    let who = bridge.pair_state().paired.map_or("not paired".to_owned(), |h| format!("paired with {h}"));
    match (turns, asks) {
        (_, a) if a > 0 => format!("Claude bridge · {who} · {a} waiting for the handheld"),
        (t, _) if t > 0 => format!("Claude bridge · {who} · {t} working"),
        _ => format!("Claude bridge · {who}"),
    }
}

/// `--claude-bridge`: the bridge in the tray, and its window when asked
/// (at once with `show`, or the first time, before anything is paired).
#[cfg(windows)]
pub fn bridge_main(show: bool) -> i32 {
    use crate::tray::{Ask, Tray};
    use std::sync::atomic::{AtomicBool, Ordering};
    if crate::tray::show_running() {
        return 0;
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("catalyst-companion.exe"));
    let config = Config::standard(state_dir(), (exe, vec!["--claude-approver".into()]), crate::win::computer_name());
    let (bridge, error) = match config.map(Bridge::start) {
        Ok(Ok(b)) => (Some(b), None),
        Ok(Err(why)) | Err(why) => (None, Some(why)),
    };
    let (tx, rx) = std::sync::mpsc::channel();
    if show || bridge.as_ref().is_none_or(|b| b.pair_state().paired.is_none()) {
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

#[cfg(not(windows))]
pub fn bridge_main(_show: bool) -> i32 {
    eprintln!("the Claude bridge runs on Windows");
    1
}

/// The window, until it is closed (the bridge carries on).
#[cfg(not(target_arch = "aarch64"))]
fn window(bridge: Option<&Bridge>, error: Option<String>) {
    use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Window as Win, WindowOptions};
    let Ok(mut win) = Win::new("Claude on the handheld · Catalyst Companion", W, H, WindowOptions::default()) else { return };
    win.set_target_fps(30);
    let mut state = Window::new(look(bridge, error.clone()));
    let (mut canvas, mut text, mut icons) = (Canvas::new(W as i32, H as i32), Text::new(), Icons::new());
    let mut focus: Option<String> = None;
    let mut was_down = false;
    let started = Instant::now();
    let mut looked_at = Instant::now();
    while win.is_open() {
        // Tailscale and the pairing looked at again every few seconds while it shows.
        if looked_at.elapsed().as_secs() >= 4 {
            let showing = state.showing.is_some();
            state.looked = look(bridge, error.clone());
            looked_at = Instant::now();
            // Paired meanwhile: the code goes, and it says so.
            if showing && state.looked.pair.as_ref().is_some_and(|p| p.offering.is_none()) {
                state.showing = None;
                if let Some(h) = state.looked.pair.as_ref().and_then(|p| p.paired.clone()) {
                    state.toast = Some((format!("paired with {h}"), false, Instant::now()));
                }
            }
        }
        let mut keys = Vec::new();
        for key in win.get_keys_pressed(KeyRepeat::No) {
            if key == Key::Escape {
                keys.push(K::Escape);
            }
        }
        let down = win.get_mouse_down(MouseButton::Left);
        let clicked = down && !was_down;
        was_down = down;
        let mouse = win.get_mouse_pos(MouseMode::Pass).unwrap_or((-1.0, -1.0));
        let mut ui = Ui { canvas: &mut canvas, text: &mut text, icons: &mut icons, t: tokens(), mouse, clicked, wheel: 0.0, typed: Vec::new(), keys, focus: &mut focus, fields: Vec::new(), pointing: false, time: started.elapsed().as_secs_f32(), pasted: None };
        state.frame(&mut ui, bridge);
        win.update_with_buffer(&canvas.px, W, H).ok();
    }
    if state.showing.is_some() {
        if let Some(b) = bridge {
            b.end_pairing();
        }
    }
}

#[cfg(target_arch = "aarch64")]
fn window(_: Option<&Bridge>, _: Option<String>) {}

/// `--claude-shot DIR`: the window as PNGs, for a made-up computer.
pub fn shots(dir: PathBuf) {
    use catalyst_claude_bridge::Key;
    std::fs::create_dir_all(&dir).expect("the folder");
    let ts = |address: bool| tailscale::Tailscale { installed: true, running: true, dns_name: Some("laptop.tail1234.ts.net".into()), address: address.then(|| "https://laptop.tail1234.ts.net:10000".into()), taken: vec![443, 8443] };
    let (mut canvas, mut text, mut icons) = (Canvas::new(W as i32, H as i32), Text::new(), Icons::new());
    let mut focus = None;
    let mut draw = |w: &mut Window, name: &str| {
        let mut ui = Ui { canvas: &mut canvas, text: &mut text, icons: &mut icons, t: tokens(), mouse: (-1.0, -1.0), clicked: false, wheel: 0.0, typed: Vec::new(), keys: Vec::new(), focus: &mut focus, fields: Vec::new(), pointing: false, time: 0.2, pasted: None };
        w.frame(&mut ui, None);
        crate::save_png_size(&canvas, &dir.join(format!("{name}.png")), W, H);
    };
    let base = Looked { claude: Some("claude.exe".into()), signed_in: Some(true), tailscale: ts(false), pair: Some(PairState { paired: None, offering: None }), busy: (0, 0), running: true, error: None };
    let mut w = Window::new(base.clone());
    draw(&mut w, "claude-1-steps");
    w.looked.tailscale = ts(true);
    draw(&mut w, "claude-2-ready");
    let invite = Invite { address: "https://laptop.tail1234.ts.net:10000".into(), key: Key::from_bytes([0x5d; 32]), computer: "THOMAS-LAPTOP".into() };
    let modules = qr_modules(&invite);
    w.showing = Some(Showing { invite, code: "482913".into(), modules, began: Instant::now() - std::time::Duration::from_secs(48) });
    draw(&mut w, "claude-3-pairing");
    w.showing = None;
    w.looked.pair = Some(PairState { paired: Some("catalyst".into()), offering: None });
    draw(&mut w, "claude-4-paired");
    println!("claude window in {}", dir.display());
}
