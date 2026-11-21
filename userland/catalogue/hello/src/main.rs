//! Hello: Catalyst Kit's example, the app to copy when starting one.
//!
//! Four pages show the kit's pieces: a list of rows with the focus ring
//! gliding over it, a message in the island, a text field whose words are
//! kept between runs, pages that slide in and out, and Detent's springs
//! moving something on their own.

use catalyst_kit::prelude::*;

const ROWS: [(&str, &str); 4] = [("front_hand", "Say hello"), ("person", "Your name"), ("animation", "Springs"), ("info", "About this app")];

/// The first page: a list.
struct Hello {
    list: List,
    saved: Saved,
}

impl Hello {
    fn new() -> Hello {
        Hello { list: List::default(), saved: Saved::open("settings") }
    }

    fn name(&self) -> &str {
        self.saved.get("name").filter(|n| !n.is_empty()).unwrap_or("there")
    }
}

impl Page for Hello {
    fn title(&self) -> &str {
        "Hello"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Flow {
        match key {
            Key::Back => Flow::Quit,
            Key::Enter => match self.list.focus {
                0 => {
                    cx.say(format!("hello, {}", self.name()));
                    Flow::Stay
                }
                1 => Flow::Push(Box::new(Name { field: Field::new(self.saved.get("name").unwrap_or("")).with_limit(20) })),
                2 => Flow::Push(Box::new(Springs::new())),
                _ => Flow::Push(Box::new(About)),
            },
            other => {
                self.list.key(other, ROWS.len());
                Flow::Stay
            }
        }
    }

    /// Back from the name page: read what it kept.
    fn resume(&mut self, _cx: &mut Cx) {
        self.saved = Saved::open("settings");
    }

    fn draw(&mut self, ui: &mut Ui) {
        let (top, h, bottom) = (ui.top(), ui.row_h(), ui.bottom());
        let name = self.name().to_owned();
        for (i, y) in self.list.layout(ui, ROWS.len(), top, h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, h);
            let (icon, label) = ROWS[i];
            let value = if i == 1 { name.as_str() } else { "" };
            ui.icon_row(r, icon, label, value, i == self.list.focus);
        }
        self.list.end(ui, ROWS.len(), top, bottom);
        ui.hint_any(&["enter: open · esc: leave", "enter · esc"]);
    }
}

/// A text field, kept in the app's own folder when Enter is pressed.
struct Name {
    field: Field,
}

impl Page for Name {
    fn title(&self) -> &str {
        "Your name"
    }

    fn wants_text(&self) -> bool {
        true
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Flow {
        match key {
            Key::Back => Flow::Pop,
            Key::Enter => {
                let mut saved = Saved::open("settings");
                saved.set("name", self.field.text.trim());
                match saved.save() {
                    Ok(()) => cx.say("kept"),
                    Err(_) => cx.say("could not keep it"),
                }
                Flow::Pop
            }
            other => {
                self.field.key(other);
                Flow::Stay
            }
        }
    }

    fn draw(&mut self, ui: &mut Ui) {
        let (t, ty) = (ui.tokens, ui.ty);
        let top = ui.top() + 4.0;
        let used = ui.paragraph(PAD + 2.0, top, W - 2.0 * PAD, "What should Hello call you? It is kept for next time.", ty.body, t.dim, 2);
        let r = Rect::new(PAD, top + used + 8.0, W - 2.0 * PAD, ui.row_h() + 4.0);
        self.field.draw(ui, r, "your name", true);
        ui.hint_any(&["enter: keep · esc: back", "enter · esc"]);
    }
}

/// Three dots crossing the page on three of Detent's springs.
struct Springs {
    dots: [Animated; 3],
    right: bool,
}

const SPRINGS: [&str; 3] = ["detent", "release", "smooth"];

impl Springs {
    fn new() -> Springs {
        Springs { dots: [Animated::new(0.0, role::detent()), Animated::new(0.0, role::release()), Animated::new(0.0, role::smooth())], right: false }
    }

    fn send(&mut self, right: bool, calm: bool) {
        self.right = right;
        for dot in &mut self.dots {
            if calm {
                dot.jump(if right { 1.0 } else { 0.0 });
            } else {
                dot.set(if right { 1.0 } else { 0.0 });
            }
        }
    }
}

impl Page for Springs {
    fn title(&self) -> &str {
        "Springs"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Flow {
        match key {
            Key::Back => return Flow::Pop,
            Key::Enter => {
                let to = !self.right;
                self.send(to, cx.calm());
            }
            Key::Right => self.send(true, cx.calm()),
            Key::Left => self.send(false, cx.calm()),
            _ => {}
        }
        Flow::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut moving = false;
        for dot in &mut self.dots {
            moving |= dot.tick(dt);
        }
        moving
    }

    fn draw(&mut self, ui: &mut Ui) {
        let (t, ty) = (ui.tokens, ui.ty);
        let top = ui.top() + 2.0;
        let lane_h = ((ui.bottom() - top) / 3.0).floor();
        let label_w = 62.0;
        for (i, dot) in self.dots.iter().enumerate() {
            let y = top + lane_h * i as f32;
            let track = Rect::new(PAD + label_w, y + lane_h * 0.5 - 2.0, W - 2.0 * PAD - label_w - 4.0, 4.0);
            ui.text_at(PAD + 2.0, track.cy(), Align::Left, SPRINGS[i], ty.label, t.dim);
            ui.surface(track, 2.0, t.track);
            let r = (lane_h * 0.32).min(9.0);
            let x = track.x + r + (track.w - 2.0 * r) * dot.value();
            ui.dot(x, track.cy(), r, if i == 0 { t.signal } else { t.ink });
        }
        ui.hint_any(&["enter: send them across · esc: back", "enter · esc"]);
    }
}

/// Words, wrapped to the panel.
struct About;

impl Page for About {
    fn title(&self) -> &str {
        "About this app"
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Flow {
        match key {
            Key::Back | Key::Enter => Flow::Pop,
            _ => Flow::Stay,
        }
    }

    fn draw(&mut self, ui: &mut Ui) {
        let (t, ty) = (ui.tokens, ui.ty);
        let top = ui.top() + 2.0;
        let rows = ((ui.bottom() - top) / (ty.body.size * 1.32)).floor() as usize;
        ui.paragraph(
            PAD + 2.0,
            top,
            W - 2.0 * PAD - 4.0,
            "Hello is written with Catalyst Kit, which draws an app as the shell draws its own: the same rows, type, glass and springs, in the look the handheld is wearing. Its source is the place to start an app of your own.",
            ty.body,
            t.ink,
            rows,
        );
        ui.hint("esc: back");
    }
}

fn main() {
    use Key::*;
    catalyst_kit::launch(
        Hello::new,
        &[
            ("1-list", &[Down]),
            ("2-said", &[Up, Enter]),
            ("3-name", &[Down, Enter, Char('A'), Char('d'), Char('a')]),
            ("4-springs", &[Back, Down, Enter, Enter]),
            ("5-about", &[Back, Down, Enter]),
        ],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use catalyst_kit::Runner;

    #[test]
    fn every_page_opens_and_closes() {
        std::env::set_var("CATALYST_APP_DATA", std::env::temp_dir().join(format!("hello-test-{}", std::process::id())));
        let mut app = Runner::new(Box::new(Hello::new()), Look::default());
        let frames = |app: &mut Runner| {
            for _ in 0..60 {
                if app.tick(1.0 / 60.0) {
                    app.draw();
                }
            }
        };
        frames(&mut app);
        for (row, title) in [(1, "Your name"), (2, "Springs"), (3, "About this app")] {
            app.key(Key::Home);
            for _ in 0..row {
                app.key(Key::Down);
            }
            app.key(Key::Enter);
            frames(&mut app);
            assert_eq!(app.top_title(), title);
            app.key(Key::Back);
            frames(&mut app);
            assert_eq!(app.top_title(), "Hello");
        }
        assert!(app.ui.missing_icons().is_empty(), "{:?}", app.ui.missing_icons());
        app.key(Key::Back);
        assert!(app.done(), "Esc on the first page leaves");
    }
}
