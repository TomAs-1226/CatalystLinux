//! Orbits: gravity in your pocket. A native rebuild of AstroBox, the
//! N-body sandbox by eggfly and kur1k0 (MIT) from the CardputerZero app
//! store: the same four universes and the same physics, drawn and moved
//! as the rest of Catalyst is.
//!
//! - [`sim`]: bodies pulling on one another, stepped as AstroBox steps them.
//! - [`universe`]: the four universes, set up as AstroBox sets them up.
//! - [`space`]: a universe running, on the whole panel.
//! - [`help`]: every key it answers.
//!
//! `catalyst-orbits --bench` steps the Milky Way and says how long a step takes.

mod help;
mod sim;
mod space;
mod universe;

use catalyst_kit::prelude::*;
use catalyst_kit::Theme;

use space::Space;
use universe::Universe;

/// The first page: the universes, one to a row, each with a line about it.
struct Orbits {
    list: List,
    /// How lifted each row is: 1 under the focus, 0 at rest.
    lift: [Animated; Universe::ALL.len()],
    placed: bool,
}

impl Orbits {
    fn new() -> Orbits {
        let at = || Animated::new(0.0, role::effect()).with_rest(0.01);
        Orbits { list: List::default(), lift: [at(), at(), at(), at()], placed: false }
    }

    /// A row's height: a name above a line about it.
    fn row_h(ui: &Ui) -> f32 {
        let ty = ui.ty;
        let room = ui.bottom() - ui.top();
        let n = Universe::ALL.len() as f32;
        // As tall as the page allows for all four, never less than the two
        // lines need (at the large size, the list scrolls).
        let need = (0.92 * ty.body.size + 0.95 * ty.caption.size + 2.0 + 3.0).ceil();
        ((room - (n - 1.0) * GAP_ROWS) / n).floor().max(need)
    }

    /// One universe's row, lifted under the focus as the kit's rows are.
    fn row(&self, ui: &mut Ui, r: Rect, u: Universe, i: usize, focused: bool) {
        let (t, ty) = (ui.tokens, ui.ty);
        let bezel = ui.look.theme == Theme::Bezel;
        let radius = if bezel { ui.radius(7.0).min(r.h * 0.5) } else { ui.radius(12.0).min(r.h * 0.5) };
        let lift = self.lift[i].value().clamp(0.0, 1.0);
        let press = if focused { ui.press() } else { 0.0 };
        let lifted = r.inset(-1.0 * lift + 1.5 * press);
        let corner = (radius + lift - 1.5 * press).max(0.0);
        ui.shadow(lifted, corner, lift);
        ui.surface(lifted, corner, t.surface1.mix(t.surface2, lift).mix(t.ink, 0.2 * press));
        let size = (r.h - 10.0).clamp(14.0, 20.0);
        ui.icon(u.icon(), focused, r.x + 9.0 + size * 0.5, r.cy(), size, if focused { t.ink } else { t.dim });
        // Two lines, centred in the row as a block.
        let x = r.x + 17.0 + size;
        let width = r.right() - 10.0 - x;
        let (name_style, about_style) = (ty.body, ty.caption);
        let block = 0.92 * name_style.size + 2.0 + 0.95 * about_style.size;
        let name_base = (r.y + (r.h - block) * 0.5 + 0.72 * name_style.size).round();
        let about_base = (name_base + 0.2 * name_style.size + 2.0 + 0.75 * about_style.size).round();
        let name = if bezel { u.name().to_lowercase() } else { u.name().to_owned() };
        let name = ui.text.fit(&name, &name_style, width);
        ui.text(x, name_base, &name, name_style, t.ink);
        let about = ui.text.fit(u.about(), &about_style, width);
        ui.text(x, about_base, &about, about_style, t.dim);
        if focused {
            ui.focus_ring(r, radius);
        }
    }
}

/// The gap between rows of the list.
const GAP_ROWS: f32 = 3.0;

impl Page for Orbits {
    fn title(&self) -> &str {
        "Orbits"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Flow {
        match key {
            Key::Back => Flow::Quit,
            Key::Enter => Flow::Push(Box::new(Space::new(Universe::ALL[self.list.focus], cx.calm()))),
            other => {
                self.list.key(other, Universe::ALL.len());
                Flow::Stay
            }
        }
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut moving = false;
        for lift in &mut self.lift {
            moving |= lift.tick(dt);
        }
        moving
    }

    fn draw(&mut self, ui: &mut Ui) {
        let (top, bottom) = (ui.top(), ui.bottom());
        let h = Orbits::row_h(ui);
        let n = Universe::ALL.len();
        // Each row's lift follows the focus on a spring (at once when calm).
        for (i, lift) in self.lift.iter_mut().enumerate() {
            let want = if i == self.list.focus { 1.0 } else { 0.0 };
            if !self.placed || ui.calm() {
                lift.jump(want);
            } else if lift.target() != want {
                lift.set(want);
                ui.started_moving();
            }
        }
        self.placed = true;
        let rows = self.list.layout(ui, n, top, h, GAP_ROWS, bottom);
        for (i, y) in rows {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, h);
            self.row(ui, r, Universe::ALL[i], i, i == self.list.focus);
        }
        self.list.end(ui, n, top, bottom);
        ui.hint_any(&["enter: open · esc: leave", "enter · esc"]);
    }
}

/// Step the Milky Way and say how long it takes: the whole of a frame's
/// physics at ×10 is up to 48 of these.
fn bench() {
    use std::time::Instant;
    for (u, steps) in [(Universe::MilkyWay, 20_000u32), (Universe::SolarSystem, 50_000), (Universe::FigureEight, 50_000)] {
        let mut sim = u.load();
        for _ in 0..500 {
            sim.step();
        }
        let start = Instant::now();
        for _ in 0..steps {
            sim.step();
        }
        let per = start.elapsed().as_secs_f64() / steps as f64;
        let n = sim.len();
        let pairs = n * (n - 1) / 2;
        println!(
            "{:<15} {n:>2} bodies, {pairs:>4} pairs: {:>7.2} µs a step, {:>6.3} ms for {} steps (a frame at most), {:.1} ns a pair",
            u.name(),
            per * 1e6,
            per * 1e3 * sim.config.max_steps as f64,
            sim.config.max_steps,
            per * 1e9 / pairs.max(1) as f64
        );
    }
    // A whole frame, drawn: the Milky Way at ×10, trails full.
    let mut runner = catalyst_kit::Runner::new(Box::new(Space::new(Universe::MilkyWay, false)), catalyst_kit::Look::default());
    for _ in 0..2 {
        runner.key(Key::Right);
    }
    for _ in 0..400 {
        runner.tick(1.0 / 40.0);
        runner.draw();
    }
    let frames = 400;
    let start = Instant::now();
    for _ in 0..frames {
        runner.tick(1.0 / 40.0);
        runner.draw();
    }
    let per = start.elapsed().as_secs_f64() / frames as f64;
    println!("Milky Way at ×10, stepped and drawn: {:.2} ms a frame", per * 1e3);
}

fn main() {
    if std::env::args().any(|a| a == "--bench") {
        return bench();
    }
    use Key::*;
    // A key Orbits does not answer: the gallery's way of letting time pass
    // (each key is followed by a second and a half of frames).
    const WAIT: Key = F(12);
    catalyst_kit::launch(
        Orbits::new,
        &[
            ("1-universes", &[]),
            ("2-solar-system", &[Enter, WAIT, WAIT, WAIT, WAIT]),
            ("3-zoomed-out", &[Down, Down, Down, Down, Down, Down, Down, Down, Right, Right]),
            ("4-following-earth", &[Char('r'), Tab, Tab, Tab, Tab, WAIT]),
            ("5-set-loose", &[Up, Up, WAIT, Char('n')]),
            ("6-figure-8", &[Back, Down, Enter, WAIT, WAIT, WAIT, WAIT]),
            ("7-three-body", &[Back, Down, Enter, Right, Right, Right, WAIT]),
            ("8-following-ember", &[Tab, Tab]),
            ("9-milky-way", &[Back, Down, Enter, Right, WAIT, WAIT, WAIT, WAIT, WAIT, WAIT, WAIT, WAIT]),
            ("10-paused", &[Char(' '), WAIT]),
            ("11-keys", &[Char('h')]),
        ],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use catalyst_kit::{Look, Runner};

    fn frames(app: &mut Runner, n: usize) {
        for _ in 0..n {
            if app.tick(1.0 / 60.0) {
                app.draw();
            }
        }
    }

    #[test]
    fn every_universe_opens_runs_and_answers_every_key() {
        std::env::set_var("CATALYST_APP_DATA", std::env::temp_dir().join(format!("orbits-test-{}", std::process::id())));
        for look in [Look::default(), Look { light: true, calm: true, text: catalyst_kit::TextSize::Large, ..Look::default() }] {
            let mut app = Runner::new(Box::new(Orbits::new()), look);
            frames(&mut app, 30);
            assert_eq!(app.top_title(), "Orbits");
            for (row, u) in Universe::ALL.iter().enumerate() {
                app.key(Key::Home);
                for _ in 0..row {
                    app.key(Key::Down);
                }
                app.key(Key::Enter);
                frames(&mut app, 30);
                assert_eq!(app.top_title(), u.name());
                for key in [Key::Right, Key::Right, Key::Up, Key::Up, Key::Down, Key::Tab, Key::Tab, Key::Char('n'), Key::Left, Key::Char(' '), Key::Char(' '), Key::Char('r'), Key::Tab] {
                    app.key(key);
                    frames(&mut app, 20);
                }
                app.key(Key::Char('h'));
                frames(&mut app, 30);
                assert_eq!(app.top_title(), "Keys");
                app.key(Key::Back);
                frames(&mut app, 30);
                assert_eq!(app.top_title(), u.name());
                app.key(Key::Back);
                frames(&mut app, 30);
                assert_eq!(app.top_title(), "Orbits");
            }
            assert!(app.ui.missing_icons().is_empty(), "{:?}", app.ui.missing_icons());
            app.key(Key::Back);
            assert!(app.done(), "Esc on the list leaves");
        }
    }

    #[test]
    fn a_paused_universe_asks_for_no_frames() {
        let mut app = Runner::new(Box::new(Orbits::new()), Look::default());
        frames(&mut app, 30);
        app.key(Key::Enter);
        frames(&mut app, 60);
        assert!(app.animating(), "running, it draws every frame");
        app.key(Key::Char(' '));
        // The island says "paused", then shrinks away.
        frames(&mut app, 300);
        assert!(!app.animating(), "paused and settled: no frames, no work");
        app.key(Key::Char(' '));
        frames(&mut app, 2);
        assert!(app.animating());
    }

    #[test]
    fn the_milky_way_steps_quickly() {
        let mut sim = Universe::MilkyWay.load();
        let steps = 2_000;
        let start = std::time::Instant::now();
        for _ in 0..steps {
            sim.step();
        }
        let per = start.elapsed().as_secs_f64() / steps as f64;
        println!("Milky Way: {:.2} µs a step, {:.3} ms for 48 steps", per * 1e6, per * 48e3);
        assert_eq!(sim.len(), 64, "nothing collided");
    }
}
