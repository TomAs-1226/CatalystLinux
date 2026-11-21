//! The tear-free panel: Developer options › Tear-free panel, and the question
//! the start it is tried in asks.
//!
//! M5Stack drives the CardputerZero's ST7789 on its side (MADCTL 0xA0: rows
//! and columns exchanged), so the panel refreshes its gate lines across the
//! picture's columns while frames arrive row by row, and anything moving
//! tears on a diagonal. The tear-free mode (the `cardputerzero-portrait`
//! overlay, its init sequence and the driver's tearing-effect line, patch
//! 0003) drives it upright, in its own order, at 40 refreshes a second, each
//! frame sent as the panel's scan starts again; the shell turns the picture
//! a quarter as it sends it (`platform::Turn`).
//!
//! Tried before it is kept, by `catalyst-power`: switched on, the handheld
//! restarts into it, and that start asks whether to keep it. Enter keeps it.
//! No answer within 15 seconds (or Esc) puts the panel back as it was and
//! restarts, so a picture that came up turned or mirrored, or not at all,
//! needs nobody to read it: waiting is enough. A start that finds it tried
//! and never answered (a crash, the power cut) undoes it too.

use std::path::Path;

use bezel::Rect;
use detent::{role, Animated};

use crate::gfx::{Gfx, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

const SCRIPT: &str = "/usr/sbin/catalyst-power";
const STATE: &str = "/data/catalyst/panel";
/// How long the question waits for Enter.
pub const ANSWER_WITHIN: f32 = 15.0;
/// Without a start animation to give way (`--no-intro`), when the count
/// begins anyway.
const COUNT_ANYWAY_AFTER: f32 = 6.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trial {
    /// Not on the handheld.
    Unavailable,
    Off,
    /// Set; the next start tries it.
    Pending,
    /// This start is the one it is tried in: ask.
    Trying,
    On,
    /// Tried and not kept: taken out again.
    Failed,
}

impl Trial {
    pub fn name(self) -> &'static str {
        match self {
            Trial::Unavailable => "on the handheld",
            Trial::Off => "off",
            Trial::Pending => "after a restart",
            Trial::Trying => "being tried",
            Trial::On => "on",
            Trial::Failed => "not kept",
        }
    }

    /// Switched on, or on its way to being.
    pub fn on(self) -> bool {
        matches!(self, Trial::Pending | Trial::Trying | Trial::On)
    }
}

/// Where the tear-free panel stands, from `catalyst-power`'s state file.
pub fn trial() -> Trial {
    if !Path::new(SCRIPT).exists() {
        return Trial::Unavailable;
    }
    match std::fs::read_to_string(STATE).unwrap_or_default().trim() {
        "trying 0" => Trial::Pending,
        "trying 1" => Trial::Trying,
        "on" => Trial::On,
        "failed" => Trial::Failed,
        _ => Trial::Off,
    }
}

/// `catalyst-power panel <what>`, away from the frame (it restarts the
/// handheld for `try` and `revert`).
fn run(what: &'static str) -> bool {
    if !Path::new(SCRIPT).exists() {
        return false;
    }
    std::thread::spawn(move || std::process::Command::new(SCRIPT).args(["panel", what]).status().ok());
    true
}

/// Developer options › Tear-free panel: switched on, the handheld restarts
/// into it to ask; switched off, the next start is as before.
pub fn switch(on: bool) -> Result<String, String> {
    if !Path::new(SCRIPT).exists() {
        return Err("the panel is set on the handheld".into());
    }
    if on {
        run("try");
        Ok("restarting to try the tear-free panel".into())
    } else {
        let ok = std::process::Command::new(SCRIPT).args(["panel", "off"]).status().is_ok_and(|s| s.success());
        if ok { Ok("tear-free panel off · from the next start".into()) } else { Err("could not change config.txt".into()) }
    }
}

/// Once, at start: a tried mode that was not kept is said (and forgotten).
pub fn not_kept_note() -> Option<String> {
    (trial() == Trial::Failed).then(|| {
        std::fs::write(STATE, "off\n").ok();
        "tear-free panel not kept".to_owned()
    })
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Answer {
    Waiting,
    Kept,
    GoingBack,
}

/// The question the start the mode is tried in asks, over everything.
pub struct KeepPanel {
    answer: Answer,
    /// Seconds since it was made, and since it began counting (once the
    /// start animation gave way, or a few seconds in without one).
    age: f32,
    counting: Option<f32>,
    /// Seconds since it was answered.
    answered: f32,
    shown: Animated,
}

impl KeepPanel {
    pub fn new(calm: bool) -> KeepPanel {
        let mut shown = Animated::new(0.0, role::smooth()).with_rest(0.004);
        if calm {
            shown.jump(1.0);
        } else {
            shown.set(1.0);
        }
        KeepPanel { answer: Answer::Waiting, age: 0.0, counting: None, answered: 0.0, shown }
    }

    /// Seconds left to answer.
    fn left(&self) -> f32 {
        (ANSWER_WITHIN - self.counting.unwrap_or(0.0)).max(0.0)
    }

    fn go_back(&mut self) {
        if self.answer == Answer::Waiting {
            self.answer = Answer::GoingBack;
            run("revert");
        }
    }
}

impl Screen for KeepPanel {
    fn title(&self) -> &str {
        "Tear-free panel"
    }

    fn full_screen(&self) -> bool {
        true
    }

    fn keeps_awake(&self) -> bool {
        true
    }

    fn appear(&mut self) {
        self.counting.get_or_insert(0.0);
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        match (key, self.answer) {
            (Key::Enter, Answer::Waiting) => {
                self.answer = Answer::Kept;
                run("confirm");
                *cx.say = Some("tear-free panel kept".into());
            }
            (Key::Back | Key::Leave, Answer::Waiting) => self.go_back(),
            _ => {}
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        self.shown.tick(dt);
        self.age += dt;
        if self.counting.is_none() && self.age >= COUNT_ANYWAY_AFTER {
            self.counting = Some(0.0);
        }
        match self.answer {
            Answer::Waiting => {
                if let Some(c) = &mut self.counting {
                    *c += dt;
                }
                if self.left() <= 0.0 {
                    self.go_back();
                }
            }
            _ => self.answered += dt,
        }
        true
    }

    fn finished(&self) -> bool {
        self.answer == Answer::Kept && self.answered > 1.0
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let a = self.shown.value().clamp(0.0, 1.0);
        g.canvas.rrect(Rect::new(0.0, 0.0, W, H), 0.0, t.ground, 1.0);
        let title = match self.answer {
            Answer::Waiting => "Keep the tear-free panel?",
            Answer::Kept => "Kept.",
            Answer::GoingBack => "Going back…",
        };
        let big = ty.display(18.0);
        let tw = g.measure(title, big);
        g.text_alpha((W - tw) * 0.5, 52.0, title, big, t.ink, a);
        let secs = self.left().ceil() as u32;
        let lines: [String; 2] = match self.answer {
            Answer::Waiting => ["Enter keeps it.".into(), format!("Turned or mirrored? Wait: back in {secs} s.")],
            Answer::Kept => ["The panel stays as it is now.".into(), String::new()],
            Answer::GoingBack => ["Restarting with the panel as it was.".into(), String::new()],
        };
        for (i, line) in lines.iter().enumerate() {
            if line.is_empty() {
                continue;
            }
            let fitted = g.text.fit(line, &ty.body, W - 2.0 * PAD);
            let w = g.measure(&fitted, ty.body);
            g.text_alpha((W - w) * 0.5, 86.0 + i as f32 * (ty.body.size + 7.0), &fitted, ty.body, t.dim, a);
        }
        // The time left, as a bar emptying: it can be read sideways too.
        if self.answer == Answer::Waiting {
            let bar = Rect::new(W * 0.5 - 70.0, 132.0, 140.0, 6.0);
            g.meter(bar, self.left() / ANSWER_WITHIN, t.signal);
        }
    }
}

/// What the question says, for the fit test.
#[cfg(test)]
const LINES: [&str; 4] = ["Enter keeps it.", "Turned or mirrored? Wait: back in 15 s.", "The panel stays as it is now.", "Restarting with the panel as it was."];

#[cfg(test)]
mod tests {
    use super::*;

    fn press(screen: &mut KeepPanel, key: Key) -> Option<String> {
        let (mut robot, mut settings, mut say) = (Robot::none(), crate::shell::Settings::default(), None);
        screen.key(key, &mut Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true });
        say
    }

    #[test]
    fn unanswered_it_goes_back_after_fifteen_seconds_from_when_it_shows() {
        let mut q = KeepPanel::new(true);
        // Under the start animation it waits; once it shows, it counts.
        q.tick(3.0);
        assert_eq!(q.counting, None);
        q.appear();
        q.tick(ANSWER_WITHIN - 0.5);
        assert_eq!(q.answer, Answer::Waiting, "half a second left");
        assert_eq!(q.left().ceil() as u32, 1);
        q.tick(1.0);
        assert_eq!(q.answer, Answer::GoingBack, "no answer: the panel goes back");
        assert!(!q.finished(), "it stays until the restart");
        // Keys after that change nothing.
        press(&mut q, Key::Enter);
        assert_eq!(q.answer, Answer::GoingBack);
    }

    #[test]
    fn enter_keeps_it_and_esc_goes_back_at_once() {
        let mut q = KeepPanel::new(true);
        q.appear();
        q.tick(4.0);
        assert_eq!(press(&mut q, Key::Enter).as_deref(), Some("tear-free panel kept"));
        assert_eq!(q.answer, Answer::Kept);
        q.tick(1.5);
        assert!(q.finished());
        let mut q = KeepPanel::new(true);
        q.appear();
        press(&mut q, Key::Back);
        assert_eq!(q.answer, Answer::GoingBack);
    }

    #[test]
    fn with_no_start_animation_it_counts_anyway() {
        let mut q = KeepPanel::new(true);
        q.tick(COUNT_ANYWAY_AFTER + 0.1);
        assert!(q.counting.is_some());
        q.tick(ANSWER_WITHIN);
        assert_eq!(q.answer, Answer::GoingBack);
    }

    #[test]
    fn off_the_handheld_it_says_so() {
        assert_eq!(trial(), Trial::Unavailable);
        assert!(switch(true).is_err());
        assert!(!Trial::Off.on() && Trial::Pending.on() && Trial::On.on());
    }

    #[test]
    fn every_line_fits() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        for size in [crate::gfx::TextSize::Compact, crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let ty = g.ty;
            for title in ["Keep the tear-free panel?", "Kept.", "Going back…"] {
                assert!(g.measure(title, ty.display(18.0)) <= W - 2.0 * PAD, "{title} at {size:?}");
            }
            for line in LINES {
                assert!(g.measure(line, ty.body) <= W - 2.0 * PAD, "{line} at {size:?}");
            }
        }
    }
}
