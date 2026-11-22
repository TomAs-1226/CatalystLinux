//! Where frames go and keys come from: the panel and keyboard on the device,
//! a window on a development PC, or PNG files for scripted screenshots.

use std::time::Duration;

use bezel::Canvas;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Enter,
    /// Esc: leave the current screen.
    Back,
    Backspace,
    Tab,
    PageUp,
    PageDown,
    /// Fn + Backspace on the handheld: the character after the cursor.
    Delete,
    /// Fn + K and Fn + N on the handheld.
    Home,
    End,
    /// F1 to F12: Fn with the number row (F11 and F12 on O and P).
    F(u8),
    /// Fn + U and Fn + I on the handheld.
    BrightnessDown,
    BrightnessUp,
    /// The sound and music keys: Fn + A, S, D and Fn + Q, W, E on the
    /// handheld, and a USB keyboard's own.
    Media(Media),
    Char(char),
    /// A letter with Ctrl held.
    Ctrl(char),
    /// Esc held for a second: leave whatever has the screen, even a program
    /// that takes every other key for itself. (A short Esc is still `Back`.)
    Leave,
    /// Tab held for [`HOLD_TO_LOCK`]: lock the handheld, from any screen.
    /// (A short Tab is still `Tab`, sent as it is let go.)
    Lock,
}

/// Ctrl-Backspace, as [`Key::Ctrl`] carries it: the notifications. With
/// Ctrl-Space (the control centre), Ctrl-Tab (the recent apps) and
/// Ctrl-Enter (search), Ctrl and each of the keyboard's four wide keys
/// brings something down over any screen.
pub const NOTIFICATIONS: char = '\u{8}';

/// The keys for sound and music, by what they do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Media {
    Mute,
    VolumeDown,
    VolumeUp,
    PlayPause,
    Previous,
    Next,
}

impl Key {
    /// The two ways out of anything that has the whole keyboard: Ctrl-Q, or
    /// Esc held down.
    pub fn leaves(self) -> bool {
        matches!(self, Key::Ctrl('q') | Key::Leave)
    }
}

/// The symbol printed on a key of the handheld's Sym layer, by the code its
/// driver sends for it. The driver takes those codes from keys the
/// CardputerZero does not have (brackets, the keypad, Japanese keys), and
/// M5Stack's own system maps them back to the printed symbols with a console
/// keymap (`tca8418_keypad_m5stack_keymap.map`); the shell reads key codes
/// itself, so it does the same here. Only for the handheld's own keyboard: a
/// USB keyboard's `[` is still `[`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn sym_char(code: u16) -> Option<char> {
    Some(match code {
        // The number row: Sym + 1 … 0.
        26 => '!',
        27 => '@',
        39 => '#',
        40 => '$',
        41 => '%',
        43 => '^',
        51 => '&',
        52 => '*',
        53 => '(',
        94 => ')',
        // Q … P.
        55 => '~',
        69 => '`',
        70 => '_',
        71 => '-',
        72 => '+',
        73 => '=',
        74 => '[',
        75 => ']',
        76 => '{',
        77 => '}',
        // A … L (F has nothing on Sym).
        79 => ';',
        80 => ':',
        81 => '\'',
        82 => '"',
        83 => '<',
        85 => '>',
        86 => '\\',
        89 => '|',
        // V … M.
        90 => ',',
        91 => '.',
        92 => '/',
        93 => '?',
        _ => return None,
    })
}

/// The keys of the Fn layer the shell has a use for, by key code: Delete,
/// Home and End, F1 to F12, the brightness keys, and the sound and music
/// keys (Fn + A, S, D: mute, quieter, louder; Fn + Q, W, E: play or pause,
/// previous, next). (The arrows and Page Up and Down are ordinary key
/// codes, read with the rest.)
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn fn_key(code: u16) -> Option<Key> {
    Some(match code {
        111 => Key::Delete,
        102 => Key::Home,
        107 => Key::End,
        59..=68 => Key::F((code - 58) as u8),
        87 => Key::F(11),
        88 => Key::F(12),
        224 => Key::BrightnessDown,
        225 => Key::BrightnessUp,
        113 => Key::Media(Media::Mute),
        114 => Key::Media(Media::VolumeDown),
        115 => Key::Media(Media::VolumeUp),
        164 => Key::Media(Media::PlayPause),
        165 => Key::Media(Media::Previous),
        163 => Key::Media(Media::Next),
        _ => return None,
    })
}

/// How long Esc is held before it means [`Key::Leave`].
const HOLD_TO_LEAVE: Duration = Duration::from_secs(1);

/// Watches one key being held down.
#[derive(Default)]
pub struct Hold {
    since: Option<std::time::Instant>,
    fired: bool,
}

impl Hold {
    /// Tell it whether the key is down now; true, once, when it has been
    /// down long enough.
    pub fn update(&mut self, down: bool) -> bool {
        match (down, self.since) {
            (false, _) => {
                self.since = None;
                self.fired = false;
            }
            (true, None) => self.since = Some(std::time::Instant::now()),
            (true, Some(since)) if !self.fired && since.elapsed() >= HOLD_TO_LEAVE => {
                self.fired = true;
                return true;
            }
            _ => {}
        }
        false
    }
}

/// How long Tab is held before it locks the handheld. Long enough that no
/// tap gets there (a typed Tab is down 60 to 150 ms), and longer than the
/// platforms' long press (Android's 400 ms, iOS's context menu at 300 to
/// 500 ms), as this one ends somewhere a passcode may be needed to leave;
/// well short of Esc held to leave (1 s), which is the heavier act.
pub const HOLD_TO_LOCK: Duration = Duration::from_millis(600);
/// The hold's cue shows only after this: a tap never flashes it.
pub const LOCK_CUE_AFTER: Duration = Duration::from_millis(150);

/// Tab, watched for a hold. A tap is sent as Tab when the key comes up, so a
/// press is never sent and then found to have been a hold; held for
/// [`HOLD_TO_LOCK`] it is [`Key::Lock`], once, and nothing more when it
/// comes up. Repeats while it is down (the kernel's, or a second press
/// reported before the release) are nothing.
#[derive(Default)]
pub struct TabHold {
    down: Option<std::time::Instant>,
    fired: bool,
}

impl TabHold {
    pub fn press(&mut self, now: std::time::Instant) {
        if self.down.is_none() {
            self.down = Some(now);
            self.fired = false;
            set_tab_down(Some(now));
        }
    }

    /// Tab came up: a tap (Tab), unless it was a hold.
    pub fn release(&mut self) -> Option<Key> {
        let tap = self.down.take().is_some() && !self.fired;
        self.fired = false;
        set_tab_down(None);
        tap.then_some(Key::Tab)
    }

    /// The hold, once it has been down long enough.
    pub fn due(&mut self, now: std::time::Instant) -> Option<Key> {
        let down = self.down?;
        if self.fired || now.saturating_duration_since(down) < HOLD_TO_LOCK {
            return None;
        }
        self.fired = true;
        set_tab_down(None);
        Some(Key::Lock)
    }

    /// When the hold will be due, for a wait to end in time for it.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn next(&self) -> Option<std::time::Instant> {
        self.down.filter(|_| !self.fired).map(|d| d + HOLD_TO_LOCK)
    }
}

/// Since when Tab has been held towards a lock (not yet a lock, nor let go):
/// the shell draws the hold's cue from it.
#[cfg(not(test))]
static TAB_DOWN: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

#[cfg(not(test))]
fn set_tab_down(at: Option<std::time::Instant>) {
    if let Ok(mut down) = TAB_DOWN.lock() {
        *down = at;
    }
}

#[cfg(not(test))]
pub fn tab_down_since() -> Option<std::time::Instant> {
    TAB_DOWN.lock().ok()?.as_ref().copied()
}

// Each test thread its own, as the tests run side by side.
#[cfg(test)]
thread_local! {
    static TAB_DOWN: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub fn set_tab_down(at: Option<std::time::Instant>) {
    TAB_DOWN.with(|d| d.set(at));
}

#[cfg(test)]
pub fn tab_down_since() -> Option<std::time::Instant> {
    TAB_DOWN.with(std::cell::Cell::get)
}

/// How long a key is held before it repeats, and then how often.
pub const REPEAT_AFTER: Duration = Duration::from_millis(400);
pub const REPEAT_EVERY: Duration = Duration::from_millis(75);

/// Held keys repeat on the shell's own clock, the same on the handheld and in
/// the simulator: once when pressed, then after 0.4 s about 13 times a second,
/// so a list can be run through by holding a key without racing to its end.
/// Only keys that move or type repeat; Enter, Esc, Tab and the shortcuts count
/// once a press. The key pressed last is the one that repeats, as on a
/// computer. `Id` is whatever names a physical key (a key code).
pub struct Repeat<Id> {
    held: Option<(Id, Key, std::time::Instant)>,
    /// The key that is Enter, while it is down.
    enter: Option<Id>,
}

impl<Id> Default for Repeat<Id> {
    fn default() -> Self {
        Repeat { held: None, enter: None }
    }
}

/// Enter repeats too while this is set: the screen in front sends a remote's
/// button for as long as Enter is held ([`crate::shell::Screen::repeats_enter`]).
pub static ENTER_REPEATS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// When Enter went down, while it is still down.
static ENTER_DOWN: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// How long Enter has been held down, if it is: an Enter that has been down
/// longer than [`REPEAT_AFTER`] is a repeat of a press, not a new one (a
/// remote's button held, not pressed again).
pub fn enter_held_for() -> Option<Duration> {
    ENTER_DOWN.lock().ok()?.map(|at| at.elapsed())
}

impl<Id: PartialEq + Copy> Repeat<Id> {
    /// A key went down, meaning `key`.
    pub fn press(&mut self, id: Id, key: Key, now: std::time::Instant) {
        let repeats = matches!(
            key,
            Key::Up
                | Key::Down
                | Key::Left
                | Key::Right
                | Key::PageUp
                | Key::PageDown
                | Key::Backspace
                | Key::Delete
                | Key::BrightnessDown
                | Key::BrightnessUp
                | Key::Media(Media::VolumeDown | Media::VolumeUp)
                | Key::Char(_)
        ) || (key == Key::Enter && ENTER_REPEATS.load(std::sync::atomic::Ordering::Relaxed));
        self.held = repeats.then_some((id, key, now + REPEAT_AFTER));
        if key == Key::Enter {
            self.enter = Some(id);
            if let Ok(mut down) = ENTER_DOWN.lock() {
                *down = Some(now);
            }
        }
    }

    /// A key came up.
    pub fn release(&mut self, id: Id) {
        if self.held.is_some_and(|(held, ..)| held == id) {
            self.held = None;
        }
        if self.enter == Some(id) {
            self.enter = None;
            if let Ok(mut down) = ENTER_DOWN.lock() {
                *down = None;
            }
        }
    }

    /// The repeat due by `now`, if one is. One at a time: after a stall, the
    /// next comes a full interval later rather than several at once.
    pub fn due(&mut self, now: std::time::Instant) -> Option<Key> {
        let (_, key, next) = self.held.as_mut()?;
        if *next > now {
            return None;
        }
        *next = (*next + REPEAT_EVERY).max(now + REPEAT_EVERY / 2);
        Some(*key)
    }

    /// When the next repeat is due, for a wait to end in time for it (the
    /// handheld's; the simulator looks every few milliseconds anyway).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn next(&self) -> Option<std::time::Instant> {
        self.held.map(|(_, _, next)| next)
    }
}

pub trait Platform {
    /// Wait up to `timeout` for keys. `None` means the user closed the shell.
    fn poll(&mut self, timeout: Duration) -> Option<Vec<Key>>;
    /// Show a frame. Returns how much of the panel had to be sent: 0 when the
    /// frame is what is already showing, 1 for the whole panel.
    fn present(&mut self, canvas: &Canvas) -> f32;
    /// A desktop window must be fed every frame to stay responsive; the panel
    /// is only written when the picture changed.
    fn present_always(&self) -> bool {
        false
    }
    /// A line of status for whoever is developing; the device has nowhere to put one.
    fn set_status(&mut self, _text: &str) {}
    /// Something else drew on the panel (an external app): forget what was
    /// last shown, so the next frame is sent whole.
    fn invalidate(&mut self) {}
    /// The panel's framebuffer device, for a program that is handed the screen.
    fn framebuffer(&self) -> Option<String> {
        None
    }
    /// What the panel shows now, read back (an app that drew for itself
    /// left it there), the way round the shell draws. None where it cannot
    /// be read; on a computer the window shows only what the shell sent.
    fn capture(&mut self) -> Option<Vec<u32>> {
        None
    }
    /// Standby with the screen dark: the panel itself sleeps (`true`), or
    /// wakes and shows the last frame again (`false`). Nothing on a computer.
    fn panel_sleep(&mut self, _asleep: bool) {}
    /// How long the panel takes over a whole frame, where that is known (the
    /// frames are paced by it). None on a computer.
    fn panel_frame(&self) -> Option<Duration> {
        None
    }
}

/// The kernel's own time over a flush before the first pixel leaves: about a
/// millisecond (measured on the handheld, the panel's framebuffer in cached
/// memory).
pub const PANEL_FLUSH_START: Duration = Duration::from_millis(1);

/// How long the panel takes over a whole frame of `frame_bytes`, from its
/// link's speed: the bytes on the wire and the kernel's start. None when the
/// speed is not known (`link_hz` 0 or missing: the link's clock moves with
/// the core's, on a system from before the core was held).
#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
pub fn panel_frame_time(link_hz: Option<u64>, frame_bytes: usize) -> Option<Duration> {
    let hz = link_hz.filter(|hz| *hz > 0)?;
    Some(Duration::from_secs_f64(frame_bytes as f64 * 8.0 / hz as f64) + PANEL_FLUSH_START)
}

/// Never exactly one row: the kernel takes a write of one whole row of the
/// framebuffer for a write of nothing (its sum ends the row at column 0),
/// so a change that small never reached the panel. The row beside it goes
/// too (it is what the panel shows already).
#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
pub fn never_one_row((first, end): (usize, usize), height: usize) -> (usize, usize) {
    if end != first + 1 || height < 2 {
        (first, end)
    } else if end < height {
        (first, end + 1)
    } else {
        (first - 1, end)
    }
}

/// The rows that differ from the frame last shown, as first..end, and note
/// this frame as shown. Only those rows are sent to the panel.
pub fn changed_rows(shown: &mut Vec<u32>, canvas: &Canvas) -> Option<(usize, usize)> {
    let (w, h) = (canvas.w as usize, canvas.h as usize);
    if shown.len() != canvas.px.len() {
        *shown = canvas.px.clone();
        return Some((0, h));
    }
    let differs = |y: usize| shown[y * w..(y + 1) * w] != canvas.px[y * w..(y + 1) * w];
    let first = (0..h).find(|y| differs(*y))?;
    let last = (first..h).rev().find(|y| differs(*y)).unwrap_or(first);
    shown[first * w..(last + 1) * w].copy_from_slice(&canvas.px[first * w..(last + 1) * w]);
    Some((first, last + 1))
}

// ------------------------------------------------------ the panel, turned ---

/// How the 320 × 170 picture is laid into a 170 × 320 framebuffer: the
/// tear-free panel mode (Developer options › Tear-free panel), where the
/// panel is driven upright, in its own scan order, and the picture is turned
/// a quarter as it is sent. Each names which picture pixel a framebuffer
/// pixel shows.
///
/// [`Turn::FOR_THE_PANEL`] is worked out from M5Stack's landscape set-up:
/// MADCTL 0xA0 (MY|MV) puts picture pixel (x, y) on the controller's column
/// 35 + y, gate line 319 − x. The tear-free mode's MADCTL 0x00 puts
/// framebuffer pixel (px, py) on column 35 + px, gate line py. The same spot
/// on the glass is then px = y, py = 319 − x: framebuffer row py is the
/// picture's column 319 − py, read from the top. A quarter turn counter-
/// clockwise. If the handheld shows it otherwise (upside down, or mirrored),
/// `CATALYST_PANEL_TURN` tries the others without rebuilding: the picture is
/// still sent in the panel's own order whichever is chosen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Turn {
    /// Row r is picture column W−1−r, top to bottom (counter-clockwise).
    Counter,
    /// Row r is picture column r, bottom to top (clockwise): Counter upside down.
    Clockwise,
    /// Row r is picture column r, top to bottom: Counter mirrored.
    Transpose,
    /// Row r is picture column W−1−r, bottom to top: Clockwise mirrored.
    AntiTranspose,
}

impl Turn {
    pub const ALL: [Turn; 4] = [Turn::Counter, Turn::Clockwise, Turn::Transpose, Turn::AntiTranspose];
    /// The one worked out for the CardputerZero's panel (see above).
    pub const FOR_THE_PANEL: Turn = Turn::Counter;

    pub fn name(self) -> &'static str {
        match self {
            Turn::Counter => "counter",
            Turn::Clockwise => "clockwise",
            Turn::Transpose => "transpose",
            Turn::AntiTranspose => "antitranspose",
        }
    }

    pub fn from_name(name: &str) -> Option<Turn> {
        Turn::ALL.into_iter().find(|t| t.name() == name.trim())
    }

    /// The turn in use: `CATALYST_PANEL_TURN`, else the panel's own.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn chosen() -> Turn {
        std::env::var("CATALYST_PANEL_TURN").ok().and_then(|n| Turn::from_name(&n)).unwrap_or(Turn::FOR_THE_PANEL)
    }

    /// The picture pixel (x, y) shown at framebuffer pixel (px, py), for a
    /// picture `w` × `h` (the framebuffer being `h` × `w`).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn source(self, px: usize, py: usize, w: usize, h: usize) -> (usize, usize) {
        match self {
            Turn::Counter => (w - 1 - py, px),
            Turn::Clockwise => (py, h - 1 - px),
            Turn::Transpose => (py, px),
            Turn::AntiTranspose => (w - 1 - py, h - 1 - px),
        }
    }

    /// The framebuffer rows that picture columns `x0..x1` land on.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn rows_for(self, x0: usize, x1: usize, w: usize) -> (usize, usize) {
        match self {
            Turn::Counter | Turn::AntiTranspose => (w - x1, w - x0),
            Turn::Clockwise | Turn::Transpose => (x0, x1),
        }
    }
}

/// The columns that differ from the frame last shown, as first..end, and
/// note this frame as shown: turned, a run of columns is a run of the
/// panel's rows, and only those are sent.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn changed_columns(shown: &mut Vec<u32>, canvas: &Canvas) -> Option<(usize, usize)> {
    let (w, h) = (canvas.w as usize, canvas.h as usize);
    if shown.len() != canvas.px.len() {
        *shown = canvas.px.clone();
        return Some((0, w));
    }
    // The span found so far: only columns outside it can widen it.
    let mut span: Option<(usize, usize)> = None;
    for y in 0..h {
        let (was, now) = (&shown[y * w..][..w], &canvas.px[y * w..][..w]);
        if was == now {
            continue;
        }
        let differs = |x: &usize| was[*x] != now[*x];
        span = Some(match span {
            None => {
                let first = (0..w).find(differs).unwrap_or(0);
                (first, (first..w).rev().find(differs).unwrap_or(first))
            }
            Some((first, last)) => ((0..first).find(differs).unwrap_or(first), (last + 1..w).rev().find(differs).unwrap_or(last)),
        });
    }
    let (first, last) = span?;
    shown.copy_from_slice(&canvas.px);
    Some((first, last + 1))
}

/// Framebuffer rows `rows` of the picture laid in by `turn`, as the panel
/// takes them (RGB565 dithered by place, or 32-bit), each `stride` bytes
/// apart, into `out`; `column` is room for one row's pixels.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn turned_rows(canvas: &Canvas, turn: Turn, rows: (usize, usize), bytes_per_pixel: usize, stride: usize, out: &mut Vec<u8>, column: &mut Vec<u32>) {
    let (w, h) = (canvas.w as usize, canvas.h as usize);
    out.resize((rows.1 - rows.0) * stride, 0);
    for py in rows.0..rows.1 {
        column.clear();
        column.extend((0..h).map(|px| {
            let (x, y) = turn.source(px, py, w, h);
            canvas.px[y * w + x]
        }));
        let row = &mut out[(py - rows.0) * stride..][..h * bytes_per_pixel];
        match bytes_per_pixel {
            2 => bezel::simd::rgb565_row(column, py, row),
            4 => {
                for (o, p) in row.chunks_exact_mut(4).zip(column.iter()) {
                    o.copy_from_slice(&p.to_ne_bytes());
                }
            }
            _ => {}
        }
    }
}

/// A framebuffer's bytes (`width` × `height` pixels, rows `stride` bytes
/// apart, RGB565 or 32-bit) as a picture the way round the shell draws: in
/// the tear-free mode the framebuffer is upright and `turn` lays it back.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn picture_from_framebuffer(bytes: &[u8], (width, height, stride): (usize, usize, usize), bytes_per_pixel: usize, turn: Option<Turn>) -> Option<Vec<u32>> {
    if !matches!(bytes_per_pixel, 2 | 4) || bytes.len() < stride * height || stride < width * bytes_per_pixel {
        return None;
    }
    let (w, h) = if turn.is_some() { (height, width) } else { (width, height) };
    let mut picture = vec![0u32; w * h];
    for py in 0..height {
        let row = &bytes[py * stride..][..width * bytes_per_pixel];
        for (px, p) in row.chunks_exact(bytes_per_pixel).enumerate() {
            let rgb = match p {
                [a, b] => bezel::canvas::rgb565_to_rgb(u16::from_ne_bytes([*a, *b])),
                _ => u32::from_ne_bytes([p[0], p[1], p[2], p[3]]) & 0xff_ffff,
            };
            let (x, y) = turn.map_or((px, py), |turn| turn.source(px, py, w, h));
            picture[y * w + x] = rgb;
        }
    }
    Some(picture)
}

/// How long the shell waits at start for the panel's framebuffer to appear
/// (its driver loads at boot, beside the shell's own start).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const PANEL_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

/// The panel's framebuffer among those under `graphics` (the kernel's
/// `/sys/class/graphics`): the one with the panel's size, 320 × 170, or
/// 170 × 320 in the tear-free mode. Another framebuffer (the Pi's HDMI one)
/// is never the panel.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn panel_in(graphics: &std::path::Path) -> Option<String> {
    let mut found: Vec<String> = std::fs::read_dir(graphics)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| std::fs::read_to_string(e.path().join("virtual_size")).is_ok_and(|size| matches!(size.trim(), "320,170" | "170,320")))
        .map(|e| format!("/dev/{}", e.file_name().to_string_lossy()))
        .collect();
    found.sort();
    found.into_iter().next()
}

// ---------------------------------------------------------------- device ---

/// The main loop's wait, ended early from another thread: an eventfd the
/// device's poll watches beside the keyboards (-1: none, as in the
/// simulator, whose wait is short anyway).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
static WAKER: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// Something for the main loop that cannot wait out its idle look (the
/// motion sensor saw the handheld picked up): its wait ends now. This lets
/// the shell look in seldom with the screen off.
#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
pub fn wake() {
    #[cfg(target_os = "linux")]
    {
        let fd = WAKER.load(std::sync::atomic::Ordering::Relaxed);
        if fd >= 0 {
            let one = 1u64;
            unsafe { libc::write(fd, (&one as *const u64).cast(), 8) };
        }
    }
}

#[cfg(target_os = "linux")]
pub mod device {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::io::Read;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    const FBIOGET_VSCREENINFO: libc::c_ulong = 0x4600;
    const FBIOGET_FSCREENINFO: libc::c_ulong = 0x4602;

    /// The kernel framebuffer (`/dev/fb0`, the panel's DRM fbdev emulation)
    /// and every evdev keyboard.
    ///
    /// Frames are written to the framebuffer file, not through a memory map.
    /// The panel hangs off SPI and the kernel sends it only what it is told
    /// changed: a write is passed on at once, for exactly the rows written,
    /// while changes made through a map are gathered and sent on a timer,
    /// about twenty times a second, which would cap and unsettle every
    /// animation.
    pub struct Device {
        fb: File,
        fb_path: String,
        /// Esc held down means leave; whether it is down, and for how long.
        esc: Hold,
        esc_down: bool,
        /// Tab: a tap on release, a lock when held.
        tab: TabHold,
        width: usize,
        height: usize,
        stride: usize,
        bytes_per_pixel: usize,
        /// Every keyboard, by its name under `/dev/input`, and whether it is
        /// the handheld's own (whose Sym layer sends codes of its own).
        inputs: Vec<(String, File, bool)>,
        /// Tells when a keyboard is plugged in or pulled out.
        watch: Option<File>,
        shift: bool,
        ctrl: bool,
        /// The key held down, repeating on the shell's clock (the kernel's
        /// own repeats are let go), by its key code.
        repeat: Repeat<u16>,
        /// The frame the panel is showing, to find what changed.
        shown: Vec<u32>,
        rows: Vec<u8>,
        /// The tear-free panel mode: the framebuffer upright (170 × 320) and
        /// the picture turned into it this way; room for one turned row.
        turn: Option<Turn>,
        column: Vec<u32>,
        /// The kernel's text console, held in graphics mode while the shell runs.
        _console: Option<File>,
        /// How long the panel takes over a whole frame, from its driver's
        /// `link_hz`, where that is known.
        panel_frame: Option<Duration>,
    }

    /// Tell the kernel's text console to stand aside. The boot line puts it
    /// on the panel, and left in text mode it would print kernel messages
    /// over the shell's picture (which is only sent again where it changes,
    /// so they would stay). Its own keyboard handling is turned off too, so
    /// keys meant for the shell cannot switch consoles or restart the
    /// machine. Held for as long as the shell runs; restored when it stops.
    fn quiet_console() -> Option<File> {
        const KDSETMODE: libc::c_ulong = 0x4B3A;
        const KD_GRAPHICS: libc::c_ulong = 0x01;
        const KDSKBMODE: libc::c_ulong = 0x4B45;
        const K_OFF: libc::c_ulong = 0x04;
        let tty = OpenOptions::new().read(true).write(true).custom_flags(libc::O_NOCTTY).open("/dev/tty0").ok()?;
        unsafe {
            libc::ioctl(tty.as_raw_fd(), KDSETMODE as _, KD_GRAPHICS);
            libc::ioctl(tty.as_raw_fd(), KDSKBMODE as _, K_OFF);
        }
        Some(tty)
    }

    impl Drop for Device {
        fn drop(&mut self) {
            const KDSETMODE: libc::c_ulong = 0x4B3A;
            const KD_TEXT: libc::c_ulong = 0x00;
            const KDSKBMODE: libc::c_ulong = 0x4B45;
            const K_UNICODE: libc::c_ulong = 0x03;
            if let Some(tty) = &self._console {
                unsafe {
                    libc::ioctl(tty.as_raw_fd(), KDSKBMODE as _, K_UNICODE);
                    libc::ioctl(tty.as_raw_fd(), KDSETMODE as _, KD_TEXT);
                }
            }
        }
    }

    /// The panel's framebuffer device, waited for while its driver loads
    /// (`CATALYST_FB` names one outright). Never another framebuffer: drawn
    /// into the Pi's HDMI one, the shell would run, and say it is well, with
    /// the panel dark, and a system tried after an update would be kept so
    /// (0.5.5's panel driver failed to load and that is what happened). With
    /// no panel it fails instead, and the update's check sends it back to the
    /// system before.
    fn panel_framebuffer() -> std::io::Result<String> {
        if let Ok(path) = std::env::var("CATALYST_FB") {
            return Ok(path);
        }
        let deadline = std::time::Instant::now() + super::PANEL_WAIT;
        loop {
            if let Some(path) = super::panel_in(std::path::Path::new("/sys/class/graphics")) {
                return Ok(path);
            }
            if std::time::Instant::now() >= deadline {
                return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no panel (no 320 × 170 framebuffer): its driver did not load"));
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }

    /// A watch on `/dev/input`, readable whenever a keyboard comes or goes.
    fn watch_inputs() -> Option<File> {
        use std::os::fd::FromRawFd;
        unsafe {
            let fd = libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC);
            if fd < 0 {
                return None;
            }
            let file = File::from_raw_fd(fd);
            (libc::inotify_add_watch(fd, c"/dev/input".as_ptr(), libc::IN_CREATE | libc::IN_DELETE) >= 0).then_some(file)
        }
    }

    impl Device {
        /// Open every keyboard not already open, and let go of unplugged ones.
        fn find_inputs(&mut self) {
            let present: Vec<String> = std::fs::read_dir("/dev/input")
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with("event"))
                .collect();
            self.inputs.retain(|(name, ..)| present.contains(name));
            for name in present {
                if self.inputs.iter().any(|(open, ..)| *open == name) {
                    continue;
                }
                if let Ok(f) = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(format!("/dev/input/{name}")) {
                    // The handheld's keyboard, as its driver names it (`tca8418c`).
                    let own = std::fs::read_to_string(format!("/sys/class/input/{name}/device/name")).is_ok_and(|n| n.contains("tca8418"));
                    self.inputs.push((name, f, own));
                }
            }
        }

        pub fn open() -> std::io::Result<Self> {
            // The wake pipe (see `wake`), made once.
            if super::WAKER.load(std::sync::atomic::Ordering::Relaxed) < 0 {
                let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
                super::WAKER.store(fd, std::sync::atomic::Ordering::Relaxed);
            }
            let fb_path = panel_framebuffer()?;
            let fb = OpenOptions::new().read(true).write(true).open(&fb_path)?;
            // fb_var_screeninfo is 160 bytes, fb_fix_screeninfo 80 on arm64.
            let mut var = [0u32; 40];
            let mut fix = [0u8; 80];
            unsafe {
                if libc::ioctl(fb.as_raw_fd(), FBIOGET_VSCREENINFO as _, var.as_mut_ptr()) != 0
                    || libc::ioctl(fb.as_raw_fd(), FBIOGET_FSCREENINFO as _, fix.as_mut_ptr()) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
            }
            let (width, height, bpp) = (var[0] as usize, var[1] as usize, var[6] as usize);
            // line_length sits after id[16], smem_start (8), smem_len, type, type_aux, visual, 3 × u16 + pad.
            let stride = u32::from_ne_bytes(fix[48..52].try_into().unwrap()) as usize;
            // The link's speed, from the panel's driver (next to its
            // tearing_effect): known when the core clock is held.
            let name = std::path::Path::new(&fb_path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let link_hz = std::fs::read_to_string(format!("/sys/class/graphics/{name}/device/link_hz")).ok().and_then(|t| t.trim().parse::<u64>().ok());
            let panel_frame = super::panel_frame_time(link_hz, stride * height);
            let mut device = Device {
                fb,
                fb_path,
                esc: Hold::default(),
                esc_down: false,
                tab: TabHold::default(),
                width,
                height,
                stride,
                bytes_per_pixel: bpp / 8,
                inputs: Vec::new(),
                watch: watch_inputs(),
                shift: false,
                ctrl: false,
                repeat: Repeat::default(),
                shown: Vec::new(),
                rows: Vec::new(),
                // Taller than wide: the tear-free mode's upright panel.
                turn: (height > width).then(Turn::chosen),
                column: Vec::new(),
                _console: quiet_console(),
                panel_frame,
            };
            device.find_inputs();
            // A shell restarted while the screen was off finds the panel asleep.
            device.panel_sleep(false);
            Ok(device)
        }

        /// What a key code means. `own`: it came from the handheld's
        /// keyboard, whose Sym layer reuses codes other keyboards give
        /// other keys (see [`sym_char`]).
        fn translate(&mut self, code: u16, value: i32, own: bool) -> Option<Key> {
            const ROWS: [(u16, &str, &str); 4] = [
                (2, "1234567890-=", "!@#$%^&*()_+"),
                (16, "qwertyuiop[]", "QWERTYUIOP{}"),
                (30, "asdfghjkl;'`", "ASDFGHJKL:\"~"),
                (44, "zxcvbnm,./", "ZXCVBNM<>?"),
            ];
            // Alt is the talk key (`crate::voice`): held, the shell listens.
            // It types nothing, here or in a terminal.
            if crate::voice::is_talk_code(code) {
                crate::voice::set_talk_key(value != 0);
                return None;
            }
            if code == 42 || code == 54 {
                self.shift = value != 0;
                return None;
            }
            if code == 29 || code == 97 {
                self.ctrl = value != 0;
                return None;
            }
            if code == 1 {
                // Esc: once when pressed. Holding it does not repeat; held for
                // a second it becomes Leave (see `poll`).
                self.esc_down = value != 0;
                return (value == 1).then_some(Key::Back);
            }
            if value == 0 {
                return None; // releases
            }
            // Ctrl-Space (or the Opt key, where the keyboard reports one)
            // brings the control centre down: once per press, since a held
            // key repeating would bring it down and put it away in turns.
            if code == 125 || code == 126 || (code == 57 && self.ctrl) {
                return (value == 1).then_some(Key::Ctrl(' '));
            }
            // Ctrl-Enter brings Spotlight up over any screen: no program in a
            // terminal can tell it from Enter, so it takes nothing from one.
            if (code == 28 || code == 96) && self.ctrl {
                return (value == 1).then_some(Key::Ctrl('\n'));
            }
            // Ctrl-Backspace brings the notifications down, the same way: a
            // terminal is sent Backspace for it, so it takes nothing either.
            if code == 14 && self.ctrl {
                return (value == 1).then_some(Key::Ctrl(NOTIFICATIONS));
            }
            if own {
                if let Some(ch) = sym_char(code) {
                    return Some(Key::Char(ch));
                }
            }
            if let Some(key) = fn_key(code) {
                return Some(key);
            }
            Some(match code {
                1 => Key::Back,
                14 => Key::Backspace,
                // Ctrl-Tab: the recent apps (no program in a terminal can be sent it).
                15 if self.ctrl => Key::Ctrl('\t'),
                15 => Key::Tab,
                28 | 96 => Key::Enter,
                57 => Key::Char(' '),
                103 => Key::Up,
                108 => Key::Down,
                105 => Key::Left,
                106 => Key::Right,
                104 => Key::PageUp,
                109 => Key::PageDown,
                _ => {
                    let (base, plain, shifted) = ROWS.iter().find(|(base, row, _)| code >= *base && ((code - base) as usize) < row.len())?;
                    let row = if self.shift { shifted } else { plain };
                    let ch = row.as_bytes()[(code - base) as usize] as char;
                    if self.ctrl && ch.is_ascii_alphabetic() { Key::Ctrl(ch.to_ascii_lowercase()) } else { Key::Char(ch) }
                }
            })
        }
    }

    impl Platform for Device {
        fn poll(&mut self, timeout: Duration) -> Option<Vec<Key>> {
            let mut fds: Vec<libc::pollfd> =
                self.inputs.iter().map(|(_, f, _)| libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLIN, revents: 0 }).collect();
            if let Some(watch) = &self.watch {
                fds.push(libc::pollfd { fd: watch.as_raw_fd(), events: libc::POLLIN, revents: 0 });
            }
            // Another thread's wake (see `wake`): last, and only emptied.
            let waker = super::WAKER.load(std::sync::atomic::Ordering::Relaxed);
            if waker >= 0 {
                fds.push(libc::pollfd { fd: waker, events: libc::POLLIN, revents: 0 });
            }
            // Wake in time for a held key's next repeat.
            let timeout = self.repeat.next().map_or(timeout, |at| timeout.min(at.saturating_duration_since(std::time::Instant::now())));
            // And for Tab's hold to come due.
            let timeout = self.tab.next().map_or(timeout, |at| timeout.min(at.saturating_duration_since(std::time::Instant::now())));
            unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout.as_millis() as i32) };
            if waker >= 0 && fds.last().is_some_and(|fd| fd.revents & libc::POLLIN != 0) {
                let mut count = 0u64;
                unsafe { libc::read(waker, (&mut count as *mut u64).cast(), 8) };
            }
            let watched = self.inputs.len() + usize::from(self.watch.is_some());
            let mut keys = Vec::new();
            let mut events = Vec::new();
            let mut changed = false;
            for (i, fd) in fds.iter().enumerate().take(watched) {
                if i >= self.inputs.len() {
                    // The watch: a keyboard came or went. Empty it and look again below.
                    if fd.revents & libc::POLLIN != 0 {
                        let mut note = [0u8; 1024];
                        while self.watch.as_mut().is_some_and(|w| w.read(&mut note).is_ok_and(|n| n > 0)) {}
                        changed = true;
                    }
                    continue;
                }
                if fd.revents & (libc::POLLERR | libc::POLLHUP) != 0 {
                    changed = true;
                }
                if fd.revents & libc::POLLIN == 0 {
                    continue;
                }
                // struct input_event on 64-bit: 16 bytes of time, type, code, value.
                let mut buf = [0u8; 24 * 16];
                let own = self.inputs[i].2;
                while let Ok(n) = self.inputs[i].1.read(&mut buf) {
                    if n < 24 {
                        break;
                    }
                    for ev in buf[..n].chunks_exact(24) {
                        let kind = u16::from_ne_bytes([ev[16], ev[17]]);
                        if kind == 1 {
                            events.push((u16::from_ne_bytes([ev[18], ev[19]]), i32::from_ne_bytes(ev[20..24].try_into().unwrap()), own));
                        }
                    }
                }
            }
            let now = std::time::Instant::now();
            for (code, value, own) in events {
                // Tab (not Ctrl-Tab, the recent apps, sent as it goes down):
                // a tap when it comes up, a lock when held.
                if code == 15 && !(self.ctrl && value == 1) {
                    match value {
                        1 => self.tab.press(now),
                        0 => keys.extend(self.tab.release()),
                        _ => {}
                    }
                    continue;
                }
                match value {
                    // Released: modifiers and Esc note it, and a held key stops repeating.
                    0 => {
                        self.translate(code, 0, own);
                        self.repeat.release(code);
                    }
                    1 => {
                        if let Some(key) = self.translate(code, 1, own) {
                            self.repeat.press(code, key, now);
                            keys.push(key);
                        }
                    }
                    // The kernel's own repeats: the shell keeps its own pace.
                    _ => {}
                }
            }
            keys.extend(self.repeat.due(now));
            keys.extend(self.tab.due(now));
            if changed {
                self.find_inputs();
            }
            if self.esc.update(self.esc_down) {
                keys.push(Key::Leave);
            }
            Some(keys)
        }

        fn present(&mut self, canvas: &Canvas) -> f32 {
            use std::os::unix::fs::FileExt;
            // The tear-free mode: the changed columns are a run of the
            // upright panel's rows, sent turned (the driver waits for the
            // panel's blank and they go out ahead of its scan).
            if let Some(turn) = self.turn {
                let Some((x0, x1)) = changed_columns(&mut self.shown, canvas) else { return 0.0 };
                let (first, end) = turn.rows_for(x0, x1, canvas.w as usize);
                let end = end.min(self.height);
                if first >= end || self.width < canvas.h as usize {
                    return 0.0;
                }
                let (first, end) = super::never_one_row((first, end), self.height);
                turned_rows(canvas, turn, (first, end), self.bytes_per_pixel, self.stride, &mut self.rows, &mut self.column);
                self.fb.write_all_at(&self.rows, (first * self.stride) as u64).ok();
                return (end - first) as f32 / self.height.max(1) as f32;
            }
            let Some((first, end)) = changed_rows(&mut self.shown, canvas) else { return 0.0 };
            let (w, end) = (self.width.min(canvas.w as usize), end.min(self.height));
            if first >= end {
                return 0.0;
            }
            let (first, end) = super::never_one_row((first, end), self.height.min(canvas.h as usize));
            self.rows.resize((end - first) * self.stride, 0);
            for y in first..end {
                let src = &canvas.px[y * canvas.w as usize..][..w];
                let row = &mut self.rows[(y - first) * self.stride..][..w * self.bytes_per_pixel];
                match self.bytes_per_pixel {
                    // Eight pixels at a time with NEON.
                    2 => bezel::simd::rgb565_row(src, y, row),
                    4 => {
                        for (o, p) in row.chunks_exact_mut(4).zip(src) {
                            o.copy_from_slice(&p.to_ne_bytes());
                        }
                    }
                    _ => {}
                }
            }
            // One write for the changed rows: the kernel sends those rows on.
            self.fb.write_all_at(&self.rows, (first * self.stride) as u64).ok();
            (end - first) as f32 / self.height.max(1) as f32
        }

        fn invalidate(&mut self) {
            self.shown.clear();
        }

        fn panel_frame(&self) -> Option<Duration> {
            self.panel_frame
        }

        fn framebuffer(&self) -> Option<String> {
            Some(self.fb_path.clone())
        }

        /// The framebuffer read back: what an app that drew for itself left.
        fn capture(&mut self) -> Option<Vec<u32>> {
            use std::os::unix::fs::FileExt;
            let mut bytes = vec![0u8; self.stride * self.height];
            self.fb.read_exact_at(&mut bytes, 0).ok()?;
            picture_from_framebuffer(&bytes, (self.width, self.height, self.stride), self.bytes_per_pixel, self.turn)
        }

        /// Blanking the framebuffer powers the display pipe down, and the
        /// panel's driver puts the ST7789 to sleep (display off, sleep in);
        /// unblanking runs its init sequence again and sends the frame it
        /// holds, which is the last one shown.
        fn panel_sleep(&mut self, asleep: bool) {
            const FBIOBLANK: libc::c_ulong = 0x4611;
            const FB_BLANK_UNBLANK: libc::c_ulong = 0;
            const FB_BLANK_POWERDOWN: libc::c_ulong = 4;
            let level = if asleep { FB_BLANK_POWERDOWN } else { FB_BLANK_UNBLANK };
            unsafe { libc::ioctl(self.fb.as_raw_fd(), FBIOBLANK as _, level) };
        }
    }
}

// ------------------------------------------------------------ simulator ---

#[cfg(not(target_arch = "aarch64"))]
pub mod sim {
    use super::*;
    use minifb::{Key as K, Scale, Window, WindowOptions};

    /// A desktop window showing the panel at 4×.
    pub struct Sim {
        window: Window,
        shown: Vec<u32>,
        esc: Hold,
        /// Tab: a tap as it comes up, a lock when held.
        tab: TabHold,
        /// The keys down at the last look, to find the ones just pressed and
        /// let go; the held one repeats on the shell's own clock.
        down: Vec<K>,
        repeat: Repeat<K>,
        /// The frame as the panel would show it.
        panel: Vec<u32>,
    }

    impl Sim {
        pub fn open(w: usize, h: usize) -> Self {
            let mut window = Window::new("Catalyst Linux · 320×170 panel", w, h, WindowOptions { scale: Scale::X4, ..Default::default() })
                .expect("open simulator window");
            // The shell starts frames on its own beat, as it does on the device.
            window.set_target_fps(0);
            #[cfg(windows)]
            show_icon(&window);
            Sim { window, shown: Vec::new(), esc: Hold::default(), tab: TabHold::default(), down: Vec::new(), repeat: Repeat::default(), panel: Vec::new() }
        }
    }

    /// How long the computer's screen shows each picture (1000 ÷ its refresh
    /// rate), or a 60 Hz screen's when the screen cannot be asked.
    pub fn screen_frame_ms() -> f32 {
        let hz = refresh_hz().filter(|hz| (30..=500).contains(hz)).unwrap_or(60);
        1000.0 / hz as f32
    }

    #[cfg(windows)]
    fn refresh_hz() -> Option<u32> {
        #[link(name = "user32")]
        extern "system" {
            fn EnumDisplaySettingsW(device: *const u16, mode: u32, devmode: *mut u32) -> i32;
        }
        const ENUM_CURRENT_SETTINGS: u32 = u32::MAX;
        // A DEVMODEW (220 bytes) as words: dmSize is the low half of word 17
        // (byte 68), dmDisplayFrequency word 46 (byte 184).
        let mut mode = [0u32; 55];
        mode[17] = 220;
        // SAFETY: a DEVMODEW-sized, aligned buffer with its size filled in, as
        // the call asks; null names the screen this program is on.
        let ok = unsafe { EnumDisplaySettingsW(std::ptr::null(), ENUM_CURRENT_SETTINGS, mode.as_mut_ptr()) };
        (ok != 0).then_some(mode[46])
    }

    #[cfg(not(windows))]
    fn refresh_hz() -> Option<u32> {
        None
    }

    /// Catalyst's icon on the window and its taskbar button, from the
    /// program's own resources (`build.rs` puts it there; a test build has none,
    /// and keeps Windows' plain one).
    #[cfg(windows)]
    fn show_icon(window: &Window) {
        use std::ffi::c_void;
        #[link(name = "kernel32")]
        extern "system" {
            fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        }
        #[link(name = "user32")]
        extern "system" {
            fn LoadImageW(instance: *mut c_void, name: *const u16, kind: u32, cx: i32, cy: i32, flags: u32) -> *mut c_void;
            fn GetSystemMetrics(index: i32) -> i32;
            fn SendMessageW(window: *mut c_void, message: u32, wparam: usize, lparam: isize) -> isize;
        }
        const IMAGE_ICON: u32 = 1;
        const WM_SETICON: u32 = 0x0080;
        // The icon's number in the resources, as build.rs names it.
        const ICON_NAME: usize = 1;
        // (ICON_BIG, SM_CXICON) for Alt-Tab and the taskbar, (ICON_SMALL, SM_CXSMICON) for the title bar.
        let sizes = [(1usize, 11), (0usize, 49)];
        // SAFETY: plain Win32 calls on this program's own module and window;
        // a missing icon comes back null and is left alone.
        unsafe {
            let module = GetModuleHandleW(std::ptr::null());
            let hwnd = window.get_window_handle();
            for (which, metric) in sizes {
                let side = GetSystemMetrics(metric);
                let icon = LoadImageW(module, ICON_NAME as *const u16, IMAGE_ICON, side, side, 0);
                if !icon.is_null() {
                    SendMessageW(hwnd, WM_SETICON, which, icon as isize);
                }
            }
        }
    }

    impl Sim {
        /// What a key on the PC's keyboard means to the shell, with Shift and
        /// Ctrl as they are; `None` for keys it has no use for (and the
        /// modifiers themselves).
        fn meaning(k: K, shift: bool, ctrl: bool) -> Option<Key> {
            Some(match k {
                K::Up => Key::Up,
                K::Down => Key::Down,
                K::Left => Key::Left,
                K::Right => Key::Right,
                K::Enter | K::NumPadEnter if ctrl => Key::Ctrl('\n'),
                K::Enter | K::NumPadEnter => Key::Enter,
                K::Escape => Key::Back,
                K::Backspace if ctrl => Key::Ctrl(NOTIFICATIONS),
                K::Backspace => Key::Backspace,
                K::Tab if ctrl => Key::Ctrl('\t'),
                K::Tab => Key::Tab,
                K::PageUp => Key::PageUp,
                K::PageDown => Key::PageDown,
                K::Delete => Key::Delete,
                K::Home => Key::Home,
                K::End => Key::End,
                K::F1 => Key::F(1),
                K::F2 => Key::F(2),
                K::F3 => Key::F(3),
                K::F4 => Key::F(4),
                K::F5 => Key::F(5),
                K::F6 => Key::F(6),
                K::F7 => Key::F(7),
                K::F8 => Key::F(8),
                K::F9 => Key::F(9),
                K::F10 => Key::F(10),
                K::F11 => Key::F(11),
                K::F12 => Key::F(12),
                // The handheld's sound keys (Fn + A, S, D), on the keypad.
                K::NumPadPlus => Key::Media(Media::VolumeUp),
                K::NumPadMinus => Key::Media(Media::VolumeDown),
                K::NumPadAsterisk => Key::Media(Media::Mute),
                K::Space if ctrl => Key::Ctrl(' '),
                K::Space => Key::Char(' '),
                K::Period => Key::Char('.'),
                K::Minus => Key::Char('-'),
                K::Slash => Key::Char(if shift { '?' } else { '/' }),
                other => {
                    let code = other as usize;
                    let ch = if code <= K::Key9 as usize {
                        (b'0' + (code - K::Key0 as usize) as u8) as char
                    } else if (K::A as usize..=K::Z as usize).contains(&code) {
                        (b'a' + (code - K::A as usize) as u8) as char
                    } else {
                        return None;
                    };
                    if ctrl && ch.is_ascii_alphabetic() {
                        Key::Ctrl(ch)
                    } else {
                        Key::Char(if shift { ch.to_ascii_uppercase() } else { ch })
                    }
                }
            })
        }
    }

    impl Platform for Sim {
        fn poll(&mut self, timeout: Duration) -> Option<Vec<Key>> {
            if !self.window.is_open() {
                return None;
            }
            // A window has to be looked after often, so never the whole wait.
            std::thread::sleep(timeout.min(Duration::from_millis(8)));
            // Take in what the keyboard did since the last look. Which keys
            // are down is read here, every look; minifb's own "pressed" is
            // only renewed when a frame is shown, so between frames it would
            // report one press two or three times, and its repeats with it.
            self.window.update();
            let now = std::time::Instant::now();
            let shift = self.window.is_key_down(K::LeftShift) || self.window.is_key_down(K::RightShift);
            let ctrl = self.window.is_key_down(K::LeftCtrl) || self.window.is_key_down(K::RightCtrl);
            let leave = self.esc.update(self.window.is_key_down(K::Escape));
            // Tab (not Ctrl-Tab): a tap as it comes up, a lock when held.
            let tab = self.window.is_key_down(K::Tab);
            let mut keys: Vec<Key> = Vec::new();
            if tab && !self.down.contains(&K::Tab) && !ctrl {
                self.tab.press(now);
            } else if !tab && self.down.contains(&K::Tab) {
                keys.extend(self.tab.release());
            }
            // Alt held is the talk key, as on the handheld (`crate::voice`).
            crate::voice::set_talk_key(self.window.is_key_down(K::LeftAlt) || self.window.is_key_down(K::RightAlt));
            let down = self.window.get_keys();
            for gone in self.down.iter().filter(|k| !down.contains(k)) {
                self.repeat.release(*gone);
            }
            let pressed: Vec<K> = down.iter().copied().filter(|k| !self.down.contains(k)).collect();
            self.down = down;
            for k in pressed {
                if k == K::Tab && !ctrl {
                    continue;
                }
                // Esc counts once per press (held, it becomes Leave), and so
                // does Ctrl-Space, which would otherwise open and shut the
                // control centre in turns while held: neither repeats.
                if let Some(key) = Self::meaning(k, shift, ctrl) {
                    self.repeat.press(k, key, now);
                    keys.push(key);
                }
            }
            keys.extend(self.repeat.due(now));
            keys.extend(self.tab.due(now));
            if leave {
                keys.push(Key::Leave);
            }
            Some(keys)
        }

        fn present_always(&self) -> bool {
            true
        }

        fn invalidate(&mut self) {
            self.shown.clear();
        }

        fn set_status(&mut self, text: &str) {
            self.window.set_title(text);
        }

        fn present(&mut self, canvas: &Canvas) -> f32 {
            let part = changed_rows(&mut self.shown, canvas).map_or(0.0, |(first, end)| (end - first) as f32 / canvas.h as f32);
            // As the panel shows it: 16-bit colour, dithered as on the device.
            let w = canvas.w as usize;
            self.panel.clear();
            self.panel.extend(canvas.px.iter().enumerate().map(|(i, &p)| bezel::canvas::rgb565_to_rgb(bezel::canvas::rgb565_dithered(p, i % w, i / w))));
            self.window.update_with_buffer(&self.panel, w, canvas.h as usize).ok();
            part
        }
    }

    /// Saves the panel as a PNG, enlarged so single pixels can be inspected.
    pub fn save_png(canvas: &Canvas, path: &std::path::Path, scale: usize) {
        let (w, h) = (canvas.w as usize, canvas.h as usize);
        let mut rgb = vec![0u8; w * scale * h * scale * 3];
        for y in 0..h * scale {
            for x in 0..w * scale {
                let p = canvas.px[(y / scale) * w + x / scale];
                rgb[(y * w * scale + x) * 3..][..3].copy_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
            }
        }
        let file = std::io::BufWriter::new(std::fs::File::create(path).expect("create screenshot"));
        let mut enc = png::Encoder::new(file, (w * scale) as u32, (h * scale) as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&rgb).unwrap();
    }

    /// Frames as an animated PNG, `delay_ms` apart, each pixel `scale`
    /// pixels square: motion that can be watched without a device.
    pub fn save_apng(frames: &[Vec<u32>], w: usize, h: usize, path: &std::path::Path, scale: usize, delay_ms: u16) {
        let file = std::io::BufWriter::new(std::fs::File::create(path).expect("create animation"));
        let mut enc = png::Encoder::new(file, (w * scale) as u32, (h * scale) as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_animated(frames.len() as u32, 0).unwrap();
        enc.set_frame_delay(delay_ms, 1000).unwrap();
        let mut writer = enc.write_header().unwrap();
        let mut rgb = vec![0u8; w * scale * h * scale * 3];
        for frame in frames {
            for y in 0..h * scale {
                for x in 0..w * scale {
                    let p = frame[(y / scale) * w + x / scale];
                    rgb[(y * w * scale + x) * 3..][..3].copy_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
                }
            }
            writer.write_image_data(&rgb).unwrap();
        }
        writer.finish().unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn only_a_framebuffer_the_panels_size_is_the_panel() {
        let root = std::env::temp_dir().join(format!("catalyst-graphics-{}", std::process::id()));
        let fb = |name: &str, size: &str| {
            std::fs::create_dir_all(root.join(name)).unwrap();
            std::fs::write(root.join(name).join("virtual_size"), format!("{size}\n")).unwrap();
        };
        // The Pi's firmware (HDMI) framebuffer alone, as when the panel's
        // driver failed to load: no panel, rather than drawing into that.
        fb("fb0", "1920,1080");
        assert_eq!(panel_in(&root), None);
        // The panel beside it, under whatever number it was given.
        fb("fb1", "320,170");
        assert_eq!(panel_in(&root).as_deref(), Some("/dev/fb1"));
        // Turned upright for the tear-free mode, still the panel.
        std::fs::remove_dir_all(&root).unwrap();
        fb("fb0", "170,320");
        assert_eq!(panel_in(&root).as_deref(), Some("/dev/fb0"));
        std::fs::remove_dir_all(&root).ok();
        assert_eq!(panel_in(&root), None, "no framebuffers at all");
    }

    #[test]
    fn a_change_of_one_row_is_sent_with_a_row_beside_it() {
        // The kernel takes a write of exactly one whole row for none.
        assert_eq!(never_one_row((40, 41), 170), (40, 42));
        assert_eq!(never_one_row((169, 170), 170), (168, 170), "the last row: the one above");
        assert_eq!(never_one_row((0, 1), 170), (0, 2));
        // Two rows or more go as they are.
        assert_eq!(never_one_row((40, 42), 170), (40, 42));
        assert_eq!(never_one_row((0, 170), 170), (0, 170));
    }

    #[test]
    fn the_panel_takes_a_whole_frame_in_the_time_its_link_allows() {
        let full = 320 * 170 * 2;
        // At 62.5 MHz: 13.9 ms on the wire, a millisecond before it.
        let fast = panel_frame_time(Some(62_500_000), full).unwrap();
        assert!((14.8..15.0).contains(&(fast.as_secs_f32() * 1000.0)), "{fast:?}");
        // Not known: none (the link's clock moves with the core's).
        assert_eq!(panel_frame_time(Some(0), full), None);
        assert_eq!(panel_frame_time(None, full), None);
    }

    #[test]
    fn the_sym_layer_types_what_its_keys_say() {
        // Sym + 1 … 0 along the number row, as printed.
        let row: String = [26, 27, 39, 40, 41, 43, 51, 52, 53, 94].iter().map(|c| sym_char(*c).unwrap()).collect();
        assert_eq!(row, "!@#$%^&*()");
        assert_eq!(sym_char(71), Some('-'), "Sym + R");
        assert_eq!(sym_char(93), Some('?'), "Sym + M");
        // Every symbol on the layer is a different one.
        let all: Vec<char> = (0..300).filter_map(sym_char).collect();
        let mut unique = all.clone();
        unique.sort();
        unique.dedup();
        assert_eq!((all.len(), unique.len()), (32, 32));
        // The base layer's own codes are left alone: letters, digits, Space, Enter.
        for code in (2..=11).chain(16..=25).chain(30..=38).chain(44..=50).chain([14, 15, 28, 57]) {
            assert_eq!(sym_char(code), None, "code {code}");
        }
    }

    #[test]
    fn the_fn_layer_gives_delete_home_end_and_f_keys() {
        assert_eq!(fn_key(111), Some(Key::Delete));
        assert_eq!((fn_key(102), fn_key(107)), (Some(Key::Home), Some(Key::End)));
        assert_eq!((fn_key(59), fn_key(68), fn_key(88)), (Some(Key::F(1)), Some(Key::F(10)), Some(Key::F(12))));
        assert_eq!(fn_key(225), Some(Key::BrightnessUp));
        assert_eq!(fn_key(103), None, "the arrows are read with the rest");
    }

    #[test]
    fn the_fn_layer_gives_the_sound_and_music_keys() {
        // As M5Stack's keymap has them: Fn + A, S, D and Fn + Q, W, E.
        assert_eq!((fn_key(113), fn_key(114), fn_key(115)), (Some(Key::Media(Media::Mute)), Some(Key::Media(Media::VolumeDown)), Some(Key::Media(Media::VolumeUp))));
        assert_eq!((fn_key(164), fn_key(165), fn_key(163)), (Some(Key::Media(Media::PlayPause)), Some(Key::Media(Media::Previous)), Some(Key::Media(Media::Next))));
        // Held, the volume keys run on as the arrows do; the others count once.
        let t0 = Instant::now();
        let mut held = Repeat::default();
        held.press(1u16, Key::Media(Media::VolumeUp), t0);
        assert!(held.next().is_some());
        held.press(2u16, Key::Media(Media::PlayPause), t0);
        assert!(held.next().is_none());
    }

    #[test]
    fn tab_tapped_is_tab_on_release_and_held_is_a_lock_once() {
        let t0 = std::time::Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut tab = TabHold::default();
        // A tap: nothing as it goes down, Tab as it comes up.
        tab.press(at(0));
        assert_eq!(tab.due(at(90)), None);
        assert!(tab_down_since().is_some(), "the cue can be drawn from it");
        assert_eq!(tab.release(), Some(Key::Tab));
        assert_eq!(tab_down_since(), None);
        // Down almost to the threshold, and let go: still a tap.
        tab.press(at(1000));
        assert_eq!(tab.due(at(1000 + 590)), None);
        assert_eq!(tab.release(), Some(Key::Tab));
        // Held: a lock at 0.6 s, once; its repeats and its release are nothing.
        tab.press(at(2000));
        assert_eq!(tab.next(), Some(at(2000) + HOLD_TO_LOCK));
        tab.press(at(2100));
        tab.press(at(2200));
        assert_eq!(tab.due(at(2599)), None);
        assert_eq!(tab.due(at(2600)), Some(Key::Lock));
        assert_eq!(tab_down_since(), None, "the cue is done with");
        assert_eq!(tab.due(at(3000)), None, "once");
        assert_eq!(tab.next(), None);
        tab.press(at(3100));
        assert_eq!(tab.release(), None, "no Tab after the lock");
        // And the next tap is a tap again.
        tab.press(at(4000));
        assert_eq!(tab.release(), Some(Key::Tab));
        assert!(LOCK_CUE_AFTER < HOLD_TO_LOCK && HOLD_TO_LOCK < HOLD_TO_LEAVE);
    }

    #[test]
    fn a_held_key_repeats_on_the_shells_own_clock() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut held = Repeat::default();
        held.press(1u16, Key::Down, t0);
        assert_eq!(held.due(at(100)), None, "a press counts once");
        assert_eq!(held.due(at(399)), None);
        assert_eq!(held.due(at(400)), Some(Key::Down), "held 0.4 s, it repeats");
        assert_eq!(held.due(at(420)), None);
        assert_eq!(held.due(at(475)), Some(Key::Down), "then every 75 ms");
        // Looked at late: one repeat, not a burst of them.
        assert_eq!(held.due(at(1000)), Some(Key::Down));
        assert_eq!(held.due(at(1001)), None);
        held.release(1);
        assert_eq!(held.due(at(5000)), None, "let go, it stops");
        // Enter, Esc, Tab and the shortcuts count once a press.
        for key in [Key::Enter, Key::Back, Key::Tab, Key::Ctrl(' '), Key::Ctrl('q')] {
            held.press(2, key, t0);
            assert_eq!(held.next(), None, "{key:?} does not repeat");
        }
        // The key pressed last is the one that repeats; letting go of an
        // earlier one does not stop it.
        held.press(4, Key::Up, t0);
        held.press(5, Key::Right, t0);
        held.release(4);
        assert_eq!(held.due(at(400)), Some(Key::Right));
        // Over two seconds held, a list moves about 25 rows, not to its end at once.
        let mut held = Repeat::default();
        held.press(6u16, Key::Down, t0);
        let moves = 1 + (0..2000).filter(|ms| held.due(at(*ms)).is_some()).count();
        assert!((20..=26).contains(&moves), "{moves}");
    }

    /// A picture whose every pixel says where it is: its index as an RGB565
    /// value, on whole RGB565 steps so the dither leaves it as it is.
    fn numbered(w: usize, h: usize) -> Canvas {
        let mut canvas = Canvas::new(w as i32, h as i32);
        for (i, p) in canvas.px.iter_mut().enumerate() {
            let v = i as u32;
            *p = ((v >> 11) & 31) << 19 | ((v >> 5) & 63) << 10 | (v & 31) << 3;
        }
        canvas
    }

    #[test]
    fn every_turn_shows_each_picture_pixel_once() {
        let (w, h) = (320, 170);
        for turn in Turn::ALL {
            let mut seen = vec![false; w * h];
            for py in 0..w {
                for px in 0..h {
                    let (x, y) = turn.source(px, py, w, h);
                    assert!(!seen[y * w + x], "{turn:?} shows ({x}, {y}) twice");
                    seen[y * w + x] = true;
                }
            }
            assert!(seen.iter().all(|s| *s), "{turn:?} leaves pixels out");
            assert_eq!(Turn::from_name(turn.name()), Some(turn));
            // A run of picture columns lands on exactly the rows rows_for says.
            for (x0, x1) in [(0, 1), (10, 40), (300, 320), (0, 320)] {
                let (r0, r1) = turn.rows_for(x0, x1, w);
                let rows: Vec<usize> = (0..w).filter(|&py| (x0..x1).contains(&turn.source(0, py, w, h).0)).collect();
                assert_eq!(rows, (r0..r1).collect::<Vec<_>>(), "{turn:?} {x0}..{x1}");
            }
        }
        // Worked out from MADCTL 0xA0 against 0x00: framebuffer row 0 is the
        // picture's rightmost column, its first pixel the column's top.
        assert_eq!(Turn::FOR_THE_PANEL.source(0, 0, w, h), (319, 0));
        assert_eq!(Turn::FOR_THE_PANEL.source(169, 319, w, h), (0, 169));
    }

    #[test]
    fn only_the_changed_columns_are_found() {
        let mut canvas = numbered(320, 170);
        let mut shown = Vec::new();
        assert_eq!(changed_columns(&mut shown, &canvas), Some((0, 320)), "the first frame whole");
        assert_eq!(changed_columns(&mut shown, &canvas), None, "nothing new");
        // Changes in three rows, the widest span across them.
        for (x, y) in [(50, 3), (12, 90), (200, 169), (60, 90)] {
            canvas.px[y * 320 + x] ^= 0xff;
        }
        assert_eq!(changed_columns(&mut shown, &canvas), Some((12, 201)));
        assert_eq!(changed_columns(&mut shown, &canvas), None);
        canvas.px[319] ^= 0xff;
        assert_eq!(changed_columns(&mut shown, &canvas), Some((319, 320)));
    }

    #[test]
    fn a_turned_frame_lands_in_the_framebuffer_file_as_the_panel_shows_it() {
        let (w, h) = (320usize, 170usize);
        let canvas = numbered(w, h);
        // A framebuffer file 170 wide (with a padded stride) and 320 tall.
        let stride = h * 2 + 12;
        let path = std::env::temp_dir().join(format!("catalyst-turned-fb-{}", std::process::id()));
        for turn in Turn::ALL {
            std::fs::write(&path, vec![0u8; stride * w]).unwrap();
            // Sent in two runs, as changed columns would be.
            let (mut out, mut column) = (Vec::new(), Vec::new());
            for (x0, x1) in [(0, 100), (100, 320)] {
                let (r0, r1) = turn.rows_for(x0, x1, w);
                turned_rows(&canvas, turn, (r0, r1), 2, stride, &mut out, &mut column);
                let mut fb = std::fs::read(&path).unwrap();
                fb[r0 * stride..][..out.len()].copy_from_slice(&out);
                std::fs::write(&path, fb).unwrap();
            }
            let fb = std::fs::read(&path).unwrap();
            for py in 0..w {
                for px in 0..h {
                    let v = u16::from_ne_bytes([fb[py * stride + px * 2], fb[py * stride + px * 2 + 1]]) as usize;
                    let (x, y) = turn.source(px, py, w, h);
                    assert_eq!(v, y * w + x, "{turn:?}: ({px}, {py}) should show picture ({x}, {y})");
                }
            }
        }
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn what_an_app_left_in_the_framebuffer_reads_back_the_way_round_the_shell_draws() {
        let (w, h) = (320usize, 170usize);
        // A picture whose every pixel says where it is, in colours the
        // panel's 16 bits keep exactly.
        let mut canvas = Canvas::new(w as i32, h as i32);
        for (i, p) in canvas.px.iter_mut().enumerate() {
            *p = bezel::canvas::rgb565_to_rgb(i as u16);
        }
        // Undithered, as an app would write it: the shell's own frames are dithered.
        let rgb565 = |p: u32| ((p >> 19 & 0x1f) << 11 | (p >> 10 & 0x3f) << 5 | (p >> 3 & 0x1f)) as u16;
        // As the panel's own mode lays it out, rows padded.
        let stride = w * 2 + 8;
        let mut fb = vec![0u8; stride * h];
        for y in 0..h {
            for x in 0..w {
                fb[y * stride + x * 2..][..2].copy_from_slice(&rgb565(canvas.px[y * w + x]).to_ne_bytes());
            }
        }
        assert_eq!(picture_from_framebuffer(&fb, (w, h, stride), 2, None).as_deref(), Some(canvas.px.as_slice()));
        // Upright in the tear-free mode, turned each way.
        for turn in Turn::ALL {
            let stride = h * 2 + 12;
            let mut fb = vec![0u8; stride * w];
            for py in 0..w {
                for px in 0..h {
                    let (x, y) = turn.source(px, py, w, h);
                    fb[py * stride + px * 2..][..2].copy_from_slice(&rgb565(canvas.px[y * w + x]).to_ne_bytes());
                }
            }
            assert_eq!(picture_from_framebuffer(&fb, (h, w, stride), 2, Some(turn)).as_deref(), Some(canvas.px.as_slice()), "{turn:?}");
        }
        // 32-bit, and a framebuffer too short to be the panel's.
        let fb: Vec<u8> = canvas.px.iter().flat_map(|p| (p | 0xff00_0000).to_ne_bytes()).collect();
        assert_eq!(picture_from_framebuffer(&fb, (w, h, w * 4), 4, None).as_deref(), Some(canvas.px.as_slice()));
        assert_eq!(picture_from_framebuffer(&fb[..100], (w, h, w * 4), 4, None), None);
    }
}
