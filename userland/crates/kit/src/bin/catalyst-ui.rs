//! `catalyst-ui`: the handheld's own screens for a shell script. An app
//! that is a script (`screen = own` in its `app` file) asks for a page and
//! gets the answer on its standard output, the way `dialog` works on a
//! computer, except that each page is drawn as the shell draws its own.
//!
//! ```text
//! catalyst-ui menu TITLE ITEM...          the item chosen
//! catalyst-ui input TITLE [PROMPT] [--value TEXT]
//!                                         the words typed
//! catalyst-ui confirm TITLE QUESTION [--yes WORD] [--no WORD]
//!                                         nothing: the exit status says
//! catalyst-ui message TITLE TEXT          until Enter or Esc
//! catalyst-ui view TITLE [FILE]           a file (or what comes in) to read
//! ```
//!
//! An item may carry an icon, `timer:Start the timer`, from the shell's
//! symbols. The exit status is 0 for an answer (or yes), 1 for Esc (or no),
//! 2 for a mistake in how it was asked.

use std::io::Read;
use std::sync::Mutex;

use catalyst_kit::prelude::*;

static ANSWER: Mutex<Option<String>> = Mutex::new(None);

fn answer(text: impl Into<String>) {
    *ANSWER.lock().unwrap() = Some(text.into());
}

const USAGE: &str = "catalyst-ui: the handheld's own screens for a script

  catalyst-ui menu TITLE ITEM...        prints the item chosen
  catalyst-ui input TITLE [PROMPT] [--value TEXT]
                                        prints the words typed
  catalyst-ui confirm TITLE QUESTION [--yes WORD] [--no WORD]
                                        exit status 0 for yes
  catalyst-ui message TITLE TEXT        until Enter or Esc
  catalyst-ui view TITLE [FILE]         a file, or what is piped in

An item can carry an icon: \"timer:Start the timer\".
Exit status: 0 answered, 1 Esc (or no), 2 asked wrongly.";

// ------------------------------------------------------------------ menu ---

struct Menu {
    title: String,
    items: Vec<(String, String)>,
    list: List,
}

impl Menu {
    fn new(title: &str, items: &[String]) -> Menu {
        let items = items
            .iter()
            .map(|item| match item.split_once(':') {
                Some((icon, label)) if catalyst_kit::bezel::Icons::has(icon) => (icon.to_owned(), label.to_owned()),
                _ => (String::new(), item.clone()),
            })
            .collect();
        Menu { title: title.to_owned(), items, list: List::default() }
    }
}

impl Page for Menu {
    fn title(&self) -> &str {
        &self.title
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Flow {
        match key {
            Key::Back => Flow::Quit,
            Key::Enter => {
                if let Some((_, label)) = self.items.get(self.list.focus) {
                    answer(label.clone());
                }
                Flow::Quit
            }
            // The first item starting with a letter typed.
            Key::Char(c) if c.is_alphanumeric() => {
                let c = c.to_lowercase().next().unwrap_or(c);
                let n = self.items.len();
                if let Some(at) = (1..=n).map(|d| (self.list.focus + d) % n).find(|i| self.items[*i].1.to_lowercase().starts_with(c)) {
                    self.list.focus = at;
                }
                Flow::Stay
            }
            other => {
                self.list.key(other, self.items.len());
                Flow::Stay
            }
        }
    }

    fn draw(&mut self, ui: &mut Ui) {
        let (top, h, bottom) = (ui.top(), ui.row_h(), ui.bottom());
        let n = self.items.len();
        for (i, y) in self.list.layout(ui, n, top, h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, h);
            let (icon, label) = &self.items[i];
            let focused = i == self.list.focus;
            if icon.is_empty() {
                let dim = ui.tokens.dim;
                ui.row(r, label, "", dim, focused);
            } else {
                ui.icon_row(r, icon, label, "", focused);
            }
        }
        self.list.end(ui, n, top, bottom);
        ui.hint_any(&["enter: choose · esc: back", "enter · esc"]);
    }
}

// ----------------------------------------------------------------- input ---

struct Input {
    title: String,
    prompt: String,
    field: Field,
}

impl Page for Input {
    fn title(&self) -> &str {
        &self.title
    }

    fn wants_text(&self) -> bool {
        true
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Flow {
        match key {
            Key::Back => Flow::Quit,
            Key::Enter => {
                answer(self.field.text.clone());
                Flow::Quit
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
        let used = ui.paragraph(PAD + 2.0, top, W - 2.0 * PAD - 4.0, &self.prompt, ty.body, t.dim, 3);
        let h = ui.row_h() + 4.0;
        let r = Rect::new(PAD, top + used + 8.0, W - 2.0 * PAD, h);
        self.field.draw(ui, r, "type here", true);
        ui.hint_any(&["enter: done · esc: cancel · fn z c: move", "enter: done · esc: cancel", "enter · esc"]);
    }
}

// --------------------------------------------------------------- confirm ---

struct Confirm {
    title: String,
    question: String,
    yes: String,
    no: String,
    /// 0 on yes, 1 on no.
    focus: usize,
}

impl Page for Confirm {
    fn title(&self) -> &str {
        &self.title
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Flow {
        match key {
            Key::Back | Key::Char('n') => Flow::Quit,
            Key::Char('y') => {
                answer("yes");
                Flow::Quit
            }
            Key::Left | Key::Up => {
                self.focus = 0;
                Flow::Stay
            }
            Key::Right | Key::Down | Key::Tab => {
                self.focus = 1;
                Flow::Stay
            }
            Key::Enter => {
                if self.focus == 0 {
                    answer("yes");
                }
                Flow::Quit
            }
            _ => Flow::Stay,
        }
    }

    fn draw(&mut self, ui: &mut Ui) {
        let (t, ty) = (ui.tokens, ui.ty);
        let top = ui.top() + 4.0;
        ui.paragraph(PAD + 2.0, top, W - 2.0 * PAD - 4.0, &self.question, ty.name, t.ink, 4);
        let h = ui.row_h() + 6.0;
        let y = ui.bottom() - h - 2.0;
        let w = (W - 2.0 * PAD - GAP) * 0.5;
        ui.button(Rect::new(PAD, y, w, h), "check_circle", &self.yes, self.focus == 0, true);
        ui.button(Rect::new(PAD + w + GAP, y, w, h), "cancel", &self.no, self.focus == 1, false);
        ui.hint_any(&["y: yes · n or esc: no", "y · n"]);
    }
}

// -------------------------------------------------------- message & view ---

struct Reading {
    title: String,
    text: String,
    /// The first line shown.
    first: usize,
    lines: usize,
    shown: usize,
    /// How the text reads: as words (a message) or as lines kept as they are (a file).
    as_lines: bool,
}

impl Page for Reading {
    fn title(&self) -> &str {
        &self.title
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Flow {
        let last = self.lines.saturating_sub(self.shown);
        match key {
            Key::Back | Key::Enter => return Flow::Quit,
            Key::Up => self.first = self.first.saturating_sub(1),
            Key::Down => self.first = (self.first + 1).min(last),
            Key::PageUp => self.first = self.first.saturating_sub(self.shown.max(1)),
            Key::PageDown => self.first = (self.first + self.shown.max(1)).min(last),
            Key::Home => self.first = 0,
            Key::End => self.first = last,
            _ => {}
        }
        Flow::Stay
    }

    fn draw(&mut self, ui: &mut Ui) {
        let (t, ty) = (ui.tokens, ui.ty);
        let style = if self.as_lines { ty.label } else { ty.body };
        let (top, bottom) = (ui.top() + 2.0, ui.bottom());
        let width = W - 2.0 * PAD - 10.0;
        let lines: Vec<String> = if self.as_lines {
            let text = self.text.replace('\t', "    ");
            text.lines().map(|l| ui.text.fit(l, &style, width)).collect()
        } else {
            ui.wrap(&self.text, style, width)
        };
        let pitch = (style.size * 1.32).round();
        self.lines = lines.len();
        self.shown = (((bottom - top) / pitch).floor() as usize).max(1);
        self.first = self.first.min(self.lines.saturating_sub(self.shown));
        for (i, line) in lines.iter().skip(self.first).take(self.shown).enumerate() {
            ui.text_at(PAD + 2.0, top + pitch * (i as f32 + 0.5), Align::Left, line, style, t.ink);
        }
        if self.lines > self.shown {
            let track = bottom - top;
            let thumb = (track * self.shown as f32 / self.lines as f32).max(10.0);
            let y = top + (track - thumb) * self.first as f32 / (self.lines - self.shown) as f32;
            ui.surface(Rect::new(W - 5.0, y, 2.5, thumb), 1.25, t.faint);
            ui.hint_any(&["f x: scroll · enter or esc: close", "enter · esc"]);
        } else {
            ui.hint_any(&["enter or esc: close", "enter · esc"]);
        }
    }
}

// ------------------------------------------------------------------ main ---

/// The value after `--name`, taken out of the arguments.
fn option(args: &mut Vec<String>, name: &str) -> Option<String> {
    let at = args.iter().position(|a| a == name)?;
    if at + 1 >= args.len() {
        return None;
    }
    let value = args.remove(at + 1);
    args.remove(at);
    Some(value)
}

fn page(mut args: Vec<String>) -> Result<Box<dyn Page>, String> {
    let kind = if args.is_empty() { String::new() } else { args.remove(0) };
    let value = option(&mut args, "--value");
    let yes = option(&mut args, "--yes");
    let no = option(&mut args, "--no");
    let title = args.first().cloned().unwrap_or_default();
    let rest = args.get(1..).unwrap_or(&[]);
    Ok(match kind.as_str() {
        "menu" if !rest.is_empty() => Box::new(Menu::new(&title, rest)),
        "input" if !title.is_empty() => {
            Box::new(Input { title, prompt: rest.join(" "), field: Field::new(value.as_deref().unwrap_or("")) })
        }
        "confirm" if !rest.is_empty() => Box::new(Confirm {
            title,
            question: rest.join(" "),
            yes: yes.unwrap_or_else(|| "Yes".into()),
            no: no.unwrap_or_else(|| "No".into()),
            focus: 0,
        }),
        "message" if !rest.is_empty() => Box::new(Reading { title, text: rest.join(" ").replace("\\n", "\n"), first: 0, lines: 0, shown: 0, as_lines: false }),
        "view" if !title.is_empty() => {
            let text = match rest.first().map(String::as_str) {
                Some(path) if path != "-" => std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?,
                _ => {
                    let mut text = String::new();
                    std::io::stdin().take(256 * 1024).read_to_string(&mut text).map_err(|e| e.to_string())?;
                    text
                }
            };
            Box::new(Reading { title, text, first: 0, lines: 0, shown: 0, as_lines: true })
        }
        _ => return Err(USAGE.into()),
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return;
    }
    #[cfg(not(target_arch = "aarch64"))]
    if let Some(at) = args.iter().position(|a| a == "--shots") {
        return shots(args.get(at + 1).map_or("shots", String::as_str));
    }
    let kind = args.first().cloned().unwrap_or_default();
    let page = match page(args) {
        Ok(page) => page,
        Err(why) => {
            eprintln!("{why}");
            std::process::exit(2);
        }
    };
    catalyst_kit::run(page);
    match ANSWER.lock().unwrap().take() {
        Some(text) if kind == "confirm" => drop(text),
        Some(text) if kind != "message" && kind != "view" => println!("{text}"),
        Some(_) => {}
        None if kind == "message" || kind == "view" => {}
        None => std::process::exit(1),
    }
}

/// Each kind of page, for the docs.
#[cfg(not(target_arch = "aarch64"))]
fn shots(dir: &str) {
    let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let pages: [(&str, Vec<String>, &[Key]); 5] = [
        ("ui-menu", s(&["menu", "Pit timer", "timer:Start the match timer", "history:Last five matches", "battery_full:Battery check", "settings:Settings"]), &[Key::Down]),
        ("ui-input", s(&["input", "Battery", "Which battery is in the robot? Its number is on the label.", "--value", "7"]), &[]),
        ("ui-confirm", s(&["confirm", "Battery", "Battery 7 read 12.4 volts. Mark it ready for the next match?", "--yes", "Ready", "--no", "Not yet"]), &[]),
        ("ui-message", s(&["message", "Checked", "All four batteries are charged and marked. The next to go in is battery 3."]), &[]),
        ("ui-view", s(&["view", "Pit log", "/nonexistent"]), &[]),
    ];
    let dir = std::path::Path::new(dir);
    for (name, args, keys) in pages {
        let page = match page(args) {
            Ok(page) => page,
            // The view's file, written here for the picture.
            Err(_) => Box::new(Reading {
                title: "Pit log".into(),
                text: "09:12  battery 3 in, 12.6 v\n09:40  q14 played, low 11.1 v\n09:52  battery 7 in, 12.5 v\n10:31  q22 played, low 10.9 v\n10:44  battery 3 charging\n10:45  battery 7 checked".into(),
                first: 0,
                lines: 0,
                shown: 0,
                as_lines: true,
            }) as Box<dyn Page>,
        };
        if let Err(why) = catalyst_kit::app::gallery(page, Look::from_env(), &[(name, keys)], dir) {
            eprintln!("{name}: {why}");
            std::process::exit(1);
        }
    }
    println!("drawn to {}", dir.display());
}
