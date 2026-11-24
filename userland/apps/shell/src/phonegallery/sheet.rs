//! The phone gallery on the handheld: the QR code as large as the panel's
//! height allows, and beside it the address and six digits to type
//! instead, how many phones have the page open, and the time left. Esc
//! stops sharing at once; `+` gives it ten more minutes.
//!
//! With no network it says so, and how to get one (Enter opens Wi-Fi);
//! it looks again every second and starts by itself once there is one.
//! Stopped (time up, five wrong codes), it says why, and Enter shares
//! again with a new code.
//!
//! What it shows arrives on Bezel's `release` spring (a short rise as it
//! fades in), and a phone arriving swells its dot on the same; the time
//! left glides back up when renewed. Every spring turns from where it is,
//! and the calm version lands at once.

use std::path::PathBuf;

use bezel::Rect;
use detent::{role, Animated};

use super::{Gallery, Look, Stopped, Trouble};
use crate::gfx::{Gfx, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// How often it looks for a network while there is none, in seconds.
const LOOK_EVERY: f32 = 1.0;
/// How far a page rises as it arrives.
const RISE: f32 = 6.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Showing {
    Sharing,
    NoNetwork,
    Stopped(Stopped),
    Trouble(Trouble),
}

pub struct GalleryScreen {
    album: PathBuf,
    look: Look,
    gallery: Option<Gallery>,
    pub(super) showing: Showing,
    /// 0 → 1 each time what it shows changes.
    arrive: Animated,
    /// Above 0 as a phone arrives: the dot swells, and settles.
    pulse: Animated,
    /// The time left as a share of ten minutes, gliding up when renewed.
    left: Animated,
    phones: usize,
    sending: bool,
    /// The seconds left as last drawn.
    second: u64,
    looked: f32,
}

impl GalleryScreen {
    /// Share the album, dressed as the handheld is now.
    pub fn new(settings: &crate::shell::Settings) -> Self {
        Self::with(crate::photos::dir(), Look::of(settings))
    }

    pub(super) fn with(album: PathBuf, look: Look) -> Self {
        let mut s = Self::idle(album, look);
        s.start();
        s
    }

    fn idle(album: PathBuf, look: Look) -> Self {
        GalleryScreen {
            album,
            look,
            gallery: None,
            showing: Showing::NoNetwork,
            arrive: Animated::new(0.0, role::release()).with_rest(0.002),
            pulse: Animated::new(0.0, role::release()).with_rest(0.002),
            left: Animated::new(1.0, role::settle()).with_rest(0.001),
            phones: 0,
            sending: false,
            second: 0,
            looked: 0.0,
        }
    }

    /// A screen around a gallery already serving (the screenshots and the
    /// tests, on a listener of their own).
    #[cfg(any(test, not(target_arch = "aarch64")))]
    pub(super) fn around(gallery: Gallery, album: PathBuf, look: Look) -> Self {
        let mut s = Self::idle(album, look);
        s.gallery = Some(gallery);
        s.showing = Showing::Sharing;
        s.arrive.jump(1.0);
        s
    }

    /// As it shows with no network, stopped or failing (the screenshots).
    #[cfg(any(test, not(target_arch = "aarch64")))]
    pub(super) fn showing(album: PathBuf, look: Look, showing: Showing) -> Self {
        let mut s = Self::idle(album, look);
        s.showing = showing;
        s.arrive.jump(1.0);
        s
    }

    fn start(&mut self) {
        self.gallery = None;
        let was = self.showing;
        self.showing = match Gallery::start(self.album.clone(), self.look.clone()) {
            Ok(g) => {
                self.gallery = Some(g);
                Showing::Sharing
            }
            Err(Trouble::NoNetwork) => Showing::NoNetwork,
            Err(t) => Showing::Trouble(t),
        };
        self.phones = 0;
        self.sending = false;
        self.left.jump(1.0);
        if self.showing != was || self.showing == Showing::Sharing {
            self.arrive.jump(0.0);
            self.arrive.set(1.0);
        }
    }

    fn stop(&mut self, why: Stopped) {
        if let Some(g) = self.gallery.take() {
            g.stop(why);
        }
    }

    fn page_changed(&mut self, showing: Showing) {
        self.showing = showing;
        self.arrive.jump(0.0);
        self.arrive.set(1.0);
    }

    /// "9:41".
    fn clock(secs: u64) -> String {
        format!("{}:{:02}", secs / 60, secs % 60)
    }

    fn status_words(&self) -> [String; 3] {
        let phones = |n: usize| if n == 1 { "1 phone".to_owned() } else { format!("{n} phones") };
        match (self.phones, self.sending) {
            (0, true) => ["sending photos".into(), "sending".into(), "sending".into()],
            (0, false) => ["waiting for a phone".into(), "no phone yet".into(), "waiting".into()],
            (n, true) => [format!("sending to {}", phones(n)), format!("{} · sending", phones(n)), phones(n)],
            (n, false) => [format!("{} connected", phones(n)), phones(n), phones(n)],
        }
    }

    fn draw_sharing(&mut self, g: &mut Gfx, a: f32, rise: f32) {
        let Some(gallery) = &self.gallery else { return };
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        // The code: the panel's height below the island, as Share's.
        let side = H - top - 6.0;
        let code = crate::share::draw_qr_faded(g, gallery.qr(), Rect::new(PAD, top + rise, side, side), a);
        let x = code.right() + 12.0;
        let width = W - PAD - x;
        // From the foot up: the keys, the time left, the phones.
        let hint_cy = H - 4.0 - ty.caption.size * 0.5;
        let meter_cy = hint_cy - ty.caption.size - 6.0;
        let status_cy = meter_cy - ty.caption.size - 6.0;
        let floor = status_cy - ty.caption.size * 0.5 - 4.0;
        // From the top down, as much as fits above them.
        let address_style = [ty.label, ty.caption].into_iter().find(|s| g.measure(gallery.address(), *s) <= width).unwrap_or(ty.caption);
        let digits_style = ty.display(20.0);
        let title_h = ty.name.size + 7.0;
        let words_h = ty.caption.size + 5.0;
        let address_h = address_style.size + 7.0;
        let digits_h = digits_style.size + 4.0;
        let needed = |title: bool, words: bool| (if title { title_h } else { 0.0 }) + (if words { words_h } else { 0.0 }) + address_h + digits_h;
        let room = floor - top;
        let (title, words) = if needed(true, true) <= room {
            (true, true)
        } else if needed(true, false) <= room {
            (true, false)
        } else {
            (false, false)
        };
        let mut y = code.y + 1.0;
        let mut line = 0.0;
        let mut at = |h: f32, y: &mut f32| {
            let cy = *y + h * 0.5 + rise * (1.0 + 0.3 * line);
            *y += h;
            line += 1.0;
            cy
        };
        if title {
            let cy = at(title_h, &mut y);
            let title = g.text.fit("On your phone", &ty.name, width);
            g.text_alpha(x, cy - 2.0, &title, ty.name, t.ink, a);
        }
        if words {
            let cy = at(words_h, &mut y);
            let words = crate::sheet::first_fitting(g, &["scan the code, or open", "scan, or open", "or open"], ty.caption, width);
            g.text_alpha(x, cy, words, ty.caption, t.dim, a);
        }
        let cy = at(address_h, &mut y);
        let address = g.text.fit(gallery.address(), &address_style, width);
        g.text_alpha(x, cy, &address, address_style, t.ink, a);
        let cy = at(digits_h, &mut y);
        let digits = format!("{} {}", &gallery.code()[..3], &gallery.code()[3..]);
        g.text_alpha(x, cy, &digits, digits_style, t.ink, a);

        // How many phones have it, a dot that swells as one arrives.
        let lit = self.phones > 0 || self.sending;
        let dot = if lit { g.good() } else { t.faint };
        let swell = 1.0 + 0.7 * self.pulse.value().max(0.0);
        g.canvas.circle(x + 3.5 + g.ox, status_cy + rise * 1.6, 3.0 * swell, dot, a);
        let choices = self.status_words();
        let refs: Vec<&str> = choices.iter().map(String::as_str).collect();
        let status = crate::sheet::first_fitting(g, &refs, ty.caption, width - 12.0);
        g.text_alpha(x + 12.0, status_cy + rise * 1.6, status, ty.caption, if lit { t.ink } else { t.dim }, a);

        // The time left: a track that empties, the minutes beside it.
        let secs = gallery.left().as_secs();
        let (long, short) = (format!("{} left", Self::clock(secs)), Self::clock(secs));
        let words = crate::sheet::first_fitting(g, &[long.as_str(), short.as_str()], ty.caption, width * 0.5);
        let words_w = g.measure(words, ty.caption);
        let track = Rect::new(x, meter_cy - 1.5 + rise * 1.8, (width - words_w - 8.0).max(16.0), 3.0);
        if a > 0.99 {
            g.meter(track, self.left.value(), t.signal);
        } else {
            g.canvas.rrect(track.offset(g.ox, 0.0), 1.5, t.meter, a);
        }
        crate::sheet::text_right(g, x + width + g.ox, meter_cy + rise * 1.8, words, ty.caption, t.dim, a);

        // And how it ends.
        let keys = crate::sheet::first_fitting(g, &["esc: stop sharing · +: more time", "esc: stop · +: more time", "esc: stop sharing", "esc: stop"], ty.caption, width);
        g.text_alpha(x, hint_cy, keys, ty.caption, t.dim, a);
    }

    /// A page with nothing to scan: what happened, what to do, the one
    /// thing Enter does, and the keys.
    fn draw_note(&mut self, g: &mut Gfx, a: f32, rise: f32, icon: &str, title: &str, words: &[&str], action: &str, hint: &[&str]) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let width = W - 2.0 * PAD;
        let icon_size = ty.name.size + 5.0;
        let title_cy = top + 2.0 + ty.name.size * 0.5 + rise;
        g.icon_alpha(icon, false, PAD + icon_size * 0.5, title_cy, icon_size, t.dim, a);
        let title_x = PAD + icon_size + 8.0;
        let title = g.text.fit(title, &ty.name, W - PAD - title_x);
        g.text_alpha(title_x, title_cy, &title, ty.name, t.ink, a);
        let row_h = g.row_h();
        let bottom = g.above_hint();
        let line_h = ty.body.size + 4.0;
        let words_top = top + ty.name.size + 10.0;
        // The longest telling that fits whole above the row.
        let room = bottom - row_h - 8.0 - words_top;
        let mut lines = Vec::new();
        for telling in words {
            lines = crate::help::wrap(g, telling, ty.body, width);
            if lines.len() as f32 * line_h <= room {
                break;
            }
        }
        let shown = ((room / line_h).floor().max(1.0) as usize).min(lines.len());
        let mut y = words_top + line_h * 0.5 + rise * 1.3;
        for line in lines.iter().take(shown) {
            g.text_alpha(PAD, y, line, ty.body, t.dim, a);
            y += line_h;
        }
        let row_y = (words_top + shown as f32 * line_h + 8.0).min(bottom - row_h);
        let r = Rect::new(PAD, row_y + rise * 1.6, W - 2.0 * PAD - 5.0, row_h);
        if a > 0.99 {
            g.row(r, action, "›", t.dim, true);
        } else {
            g.surface(r, g.radius(r.h * 0.5).min(r.h * 0.5), t.surface1);
        }
        g.hint_any(hint);
    }
}

impl Screen for GalleryScreen {
    fn title(&self) -> &str {
        "Phone gallery"
    }

    /// While a phone may be looking, the panel stays on.
    fn keeps_awake(&self) -> bool {
        self.showing == Showing::Sharing
    }

    /// Sharing ends when the sheet is left: it is never kept among the
    /// recent apps still serving.
    fn keep_in_recents(&self) -> bool {
        false
    }

    fn set_aside(&mut self) {
        if self.gallery.is_some() {
            self.stop(Stopped::Left);
            self.showing = Showing::Stopped(Stopped::Left);
        }
    }

    /// Back from Wi-Fi: a network may be there now.
    fn resume(&mut self) {
        if self.showing == Showing::NoNetwork {
            self.start();
        }
    }

    fn busy(&self) -> bool {
        matches!(self.showing, Showing::Sharing | Showing::NoNetwork)
    }

    fn key_sheet(&self) -> Vec<crate::help::KeyGroup> {
        use crate::help::keys;
        match self.showing {
            Showing::Sharing => vec![keys("sharing", &[("esc", "Stop sharing"), ("+", "Ten more minutes")])],
            Showing::NoNetwork => vec![keys("no network", &[("enter", "Wi-Fi"), ("esc", "Back")])],
            Showing::Stopped(_) | Showing::Trouble(_) => vec![keys("stopped", &[("enter", "Share again"), ("esc", "Back")])],
        }
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match key {
            // At once, closing whatever is on its way.
            Key::Back | Key::Leave => {
                self.stop(Stopped::ByYou);
                return Nav::Pop;
            }
            Key::Char('+' | '=') if self.showing == Showing::Sharing => {
                if let Some(g) = &self.gallery {
                    g.renew();
                    *cx.say = Some("ten more minutes".into());
                }
            }
            Key::Enter => match self.showing {
                Showing::NoNetwork => return Nav::Push(Box::new(crate::screens::WifiScreen::new())),
                Showing::Stopped(_) | Showing::Trouble(_) => self.start(),
                Showing::Sharing => {}
            },
            _ => {}
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut changed = self.arrive.tick(dt) | self.pulse.tick(dt) | self.left.tick(dt);
        match self.showing {
            Showing::Sharing => {
                let Some(g) = &self.gallery else { return changed };
                changed |= g.tick();
                if let Some(why) = g.stopped() {
                    self.gallery = None;
                    self.page_changed(Showing::Stopped(why));
                    return true;
                }
                let phones = g.phones();
                if phones > self.phones {
                    // Swells, and settles back on the release spring.
                    self.pulse.jump(1.0);
                    self.pulse.set(0.0);
                }
                changed |= phones != self.phones || g.sending() != self.sending;
                self.phones = phones;
                self.sending = g.sending();
                let left = g.left();
                let share = left.as_secs_f32() / g.open_for().as_secs_f32().max(1.0);
                if share > self.left.target() + 0.02 {
                    self.left.set(share);
                } else if self.left.is_settled() {
                    self.left.jump(share);
                }
                if left.as_secs() != self.second {
                    self.second = left.as_secs();
                    changed = true;
                }
            }
            Showing::NoNetwork => {
                self.looked += dt;
                if self.looked >= LOOK_EVERY {
                    self.looked = 0.0;
                    if crate::lan::address_on(&["wlan0", "eth0"]).is_some() {
                        self.start();
                        changed = true;
                    }
                }
            }
            _ => {}
        }
        changed
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        if g.calm {
            for s in [&mut self.arrive, &mut self.pulse, &mut self.left] {
                s.jump(s.target());
            }
        }
        let arrive = self.arrive.value();
        let a = arrive.clamp(0.0, 1.0);
        let rise = (1.0 - arrive) * RISE;
        match self.showing {
            Showing::Sharing => self.draw_sharing(g, a, rise),
            Showing::NoNetwork => self.draw_note(
                g,
                a,
                rise,
                "wifi_off",
                "No network",
                &["Join Wi-Fi, or your phone's hotspot. Sharing starts here as soon as there is one.", "Join Wi-Fi, or your phone's hotspot.", "Join Wi-Fi or a hotspot."],
                "Wi-Fi",
                &["enter: wi-fi · esc: back", "enter: wi-fi"],
            ),
            Showing::Stopped(why) => {
                let words: &[&str] = match why {
                    Stopped::TimeUp => &["Ten minutes went by without a phone looking. Share again for a new code.", "Ten minutes without a phone looking.", "Time is up."],
                    Stopped::WrongCodes => &["Five wrong codes were tried, so sharing stopped. Share again for a new code.", "Five wrong codes were tried.", "Wrong codes."],
                    Stopped::ByYou | Stopped::Left => &["Phones can no longer see the photos. Share again for a new code.", "Phones can no longer see them.", "Stopped."],
                };
                self.draw_note(g, a, rise, "stop_circle", "Sharing stopped", words, "Share again", &["enter: share again · esc: back", "enter: share again"]);
            }
            Showing::Trouble(trouble) => {
                let words: &[&str] = match trouble {
                    Trouble::NoPort => &["Another program holds the ports a phone would use (80 and 8080).", "The ports are taken."],
                    Trouble::NoThread | Trouble::NoNetwork => &["The handheld is short of memory. Close an app and try again.", "Short of memory."],
                };
                self.draw_note(g, a, rise, "error", "Couldn't start sharing", words, "Try again", &["enter: try again · esc: back", "enter: try again"]);
            }
        }
    }
}
