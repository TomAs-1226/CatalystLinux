//! Actions: shell commands kept under a name and run with one key, each in
//! a terminal of its own, its output left on screen when it is done. A few
//! come ready (the robot pinged, its agent asked how it is, the disk and
//! memory, the kernel's messages); any can be changed, removed or added.
//!
//! `{robot}` and `{team}` in a command are filled in when it runs: the
//! robot's address and the team's number.

use std::path::PathBuf;

use bezel::Rect;

use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};
use crate::tools::{Field, Typed};

/// A command under its name.
#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub name: String,
    pub command: String,
}

const DEFAULTS: [(&str, &str); 8] = [
    ("Ping the robot", "ping -c 4 {robot}"),
    ("Ask the robot's agent", "curl -s http://{robot}:9010/api/system"),
    ("Disk space", "df -h"),
    ("Memory", "free -m"),
    ("Addresses", "ip addr"),
    ("Busiest processes", "top -b -n 1 | head -n 20"),
    ("Kernel messages", "dmesg | tail -n 40"),
    ("System log", "logread | tail -n 40"),
];

fn file() -> PathBuf {
    crate::config_dir().join("commands")
}

/// The commands kept, one a line as `name<tab>command`; the ready ones until
/// the list is first changed.
pub fn load() -> Vec<Saved> {
    match std::fs::read_to_string(file()) {
        Ok(text) => text
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .filter(|(n, c)| !n.trim().is_empty() && !c.trim().is_empty())
            .map(|(n, c)| Saved { name: n.trim().to_owned(), command: c.trim().to_owned() })
            .collect(),
        Err(_) => DEFAULTS.iter().map(|(n, c)| Saved { name: (*n).into(), command: (*c).into() }).collect(),
    }
}

pub fn save(saved: &[Saved]) -> bool {
    let text: String = saved.iter().map(|s| format!("{}\t{}\n", s.name, s.command)).collect();
    std::fs::create_dir_all(crate::config_dir()).ok();
    crate::sys::write_atomic(&file(), text.as_bytes()).is_ok()
}

/// The command as it runs: `{robot}` and `{team}` filled in. `Err` when it
/// needs one that is not known yet.
pub fn expand(command: &str, robot: Option<&str>, team: Option<u32>) -> Result<String, String> {
    let mut line = command.to_owned();
    if line.contains("{robot}") {
        line = line.replace("{robot}", robot.ok_or("no robot yet · set it in Robot")?);
    }
    if line.contains("{team}") {
        line = line.replace("{team}", &team.ok_or("no team yet · set it in Robot")?.to_string());
    }
    Ok(line)
}

/// A typed command's characters: anything printable (tabs and line breaks
/// would split it in the file).
fn command_char(c: char) -> bool {
    !c.is_control()
}

// ------------------------------------------------------------------ list ---

pub struct Commands {
    saved: Vec<Saved>,
    list: ListState,
    deleting: Option<usize>,
}

impl Commands {
    pub fn new() -> Self {
        Commands { saved: load(), list: ListState::default(), deleting: None }
    }
}

impl Screen for Commands {
    fn title(&self) -> &str {
        "Actions"
    }

    fn resume(&mut self) {
        self.saved = load();
        self.list.focus = self.list.focus.min(self.saved.len());
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let armed = self.deleting.take();
        let at = self.list.focus;
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter | Key::Right if at == self.saved.len() => return Nav::Push(Box::new(Edit::new(None, None))),
            Key::Enter | Key::Right => {
                let s = &self.saved[at];
                let robot = crate::tools::robot_address(cx.robot);
                match expand(&s.command, robot.as_deref(), crate::tools::team(cx.robot)) {
                    Ok(line) => {
                        let session = crate::terminal::Terminal::running("Command", &line, crate::connect::session_dir()).with_banner(&format!("$ {line}"));
                        return Nav::Push(Box::new(session));
                    }
                    Err(why) => *cx.say = Some(why),
                }
            }
            Key::Char('n') => return Nav::Push(Box::new(Edit::new(None, None))),
            Key::Char('e') if at < self.saved.len() => return Nav::Push(Box::new(Edit::new(Some(at), Some(self.saved[at].clone())))),
            Key::Char('d') if at < self.saved.len() => {
                if armed == Some(at) {
                    let gone = self.saved.remove(at);
                    save(&self.saved);
                    *cx.say = Some(format!("removed {}", gone.name.to_lowercase()));
                } else {
                    *cx.say = Some(format!("d again removes {}", self.saved[at].name.to_lowercase()));
                    self.deleting = Some(at);
                }
            }
            other => crate::screens::list_key(&mut self.list, other, self.saved.len() + 1),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        let len = self.saved.len() + 1;
        for (i, y) in self.list.layout(g, len, top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            match self.saved.get(i) {
                Some(s) => {
                    // As much of the command as fits beside its name.
                    let style = if g.theme == crate::look::Theme::Bezel { g.ty.body } else { g.ty.label };
                    let name_w = g.measure(&s.name, g.ty.body).max(60.0);
                    let preview = g.text.fit(&s.command, &style, (r.w - name_w - 34.0).max(30.0));
                    g.row(r, &s.name, &preview, t.dim, i == self.list.focus);
                }
                None => g.row(r, "New command", "n", t.dim, i == self.list.focus),
            }
        }
        self.list.scrollbar(g, len, top, bottom);
        g.hint(if self.list.focus < self.saved.len() { "enter: run · e: edit · d: remove" } else { "enter: a new one" });
    }
}

// ------------------------------------------------------------------ edit ---

/// A command's name and line, typed one after the other.
pub struct Edit {
    /// Which command this changes, or none for a new one.
    index: Option<usize>,
    name: String,
    command: String,
    list: ListState,
    field: Option<(Field, bool)>,
}

impl Edit {
    fn new(index: Option<usize>, saved: Option<Saved>) -> Self {
        let (name, command) = saved.map_or((String::new(), String::new()), |s| (s.name, s.command));
        Edit { index, name, command, list: ListState::default(), field: None }
    }
}

impl Screen for Edit {
    fn title(&self) -> &str {
        "Command"
    }

    fn wants_text(&self) -> bool {
        self.field.is_some()
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        if let Some((field, is_name)) = &mut self.field {
            let is_name = *is_name;
            match field.key(key, command_char) {
                Typed::Done(text) => {
                    if is_name {
                        self.name = text;
                    } else {
                        self.command = text;
                    }
                    self.field = None;
                    // On to what is still to be typed, then to Keep.
                    self.list.focus = if self.command.is_empty() { 1 } else { 2 };
                }
                Typed::Cancelled => self.field = None,
                Typed::More => {}
            }
            return Nav::Stay;
        }
        match (key, self.list.focus) {
            (Key::Back, _) => return Nav::Pop,
            (Key::Enter | Key::Right, 0) => self.field = Some((Field::new("its name", &self.name, 32), true)),
            (Key::Enter | Key::Right, 1) => self.field = Some((Field::new("the command · {robot} {team} filled in", &self.command, 240), false)),
            (Key::Enter, _) => {
                if self.name.is_empty() || self.command.is_empty() {
                    *cx.say = Some("it needs a name and a command".into());
                    self.list.focus = if self.name.is_empty() { 0 } else { 1 };
                    return Nav::Stay;
                }
                let mut saved = load();
                let this = Saved { name: self.name.clone(), command: self.command.clone() };
                match self.index.filter(|i| *i < saved.len()) {
                    Some(i) => saved[i] = this,
                    None => saved.push(this),
                }
                *cx.say = Some(if save(&saved) { format!("kept {}", self.name.to_lowercase()) } else { "could not keep it".into() });
                return Nav::Pop;
            }
            (other, _) => crate::screens::list_key(&mut self.list, other, 3),
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        if let Some((field, _)) = &self.field {
            return field.draw(g, "enter: done · esc: cancel");
        }
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        let style = if g.theme == crate::look::Theme::Bezel { g.ty.body } else { g.ty.label };
        for (i, y) in self.list.layout(g, 3, top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let (name, value) = match i {
                0 => ("Name", if self.name.is_empty() { "–".to_owned() } else { self.name.clone() }),
                1 => ("Command", if self.command.is_empty() { "–".to_owned() } else { g.text.fit(&self.command, &style, r.w - 110.0) }),
                _ => ("Keep it", "›".to_owned()),
            };
            g.row(r, name, &value, if i == 2 { t.ink } else { t.dim }, i == self.list.focus);
        }
        self.list.scrollbar(g, 3, top, bottom);
        g.hint("{robot} and {team} are filled in when it runs");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_robot_and_the_team_are_filled_in() {
        assert_eq!(expand("ping -c 4 {robot}", Some("10.58.5.2"), None).unwrap(), "ping -c 4 10.58.5.2");
        assert_eq!(expand("echo {team} {robot}", Some("r"), Some(5805)).unwrap(), "echo 5805 r");
        assert!(expand("ping {robot}", None, Some(5805)).is_err(), "no robot to fill in");
        assert_eq!(expand("df -h", None, None).unwrap(), "df -h", "nothing needed");
    }

    #[test]
    fn commands_are_kept_and_the_ready_ones_come_until_then() {
        crate::shell::tests::sandbox();
        std::fs::remove_file(file()).ok();
        let ready = load();
        assert_eq!(ready.len(), DEFAULTS.len());
        assert!(ready.iter().any(|s| s.command.contains("{robot}")));
        let mine = vec![Saved { name: "Deploy".into(), command: "cd /data/code/robot && ./gradlew deploy".into() }];
        assert!(save(&mine));
        assert_eq!(load(), mine);
        // An empty list stays empty: the ready ones do not come back.
        assert!(save(&[]));
        assert!(load().is_empty());
        std::fs::remove_file(file()).ok();
    }

    #[test]
    fn a_command_is_typed_without_tabs_or_line_breaks() {
        assert!(command_char('|') && command_char('{') && command_char(' '));
        assert!(!command_char('\t') && !command_char('\n'));
    }
}
