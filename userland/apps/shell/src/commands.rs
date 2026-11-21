//! Actions: shell commands kept under a name and run with one key, each in
//! a terminal of its own, its output left on screen when it is done. The
//! team's own come first; after them, built-in ones for the robot, the
//! network and the handheld itself, which stay as they are (`e` on one keeps
//! a copy to change).
//!
//! `{robot}` and `{team}` in a command are filled in when it runs: the
//! robot's address and the team's number.
//!
//! The assistant may run them too. One that could change something (deploy
//! code, restart the robot, write a file) is critical: the assistant asks
//! before running it. Which are critical is worked out from the command, and
//! each can be told otherwise in its editor.

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
    /// Whether the assistant asks before running it.
    pub ask: Ask,
}

impl Saved {
    pub fn new(name: &str, command: &str) -> Self {
        Saved { name: name.to_owned(), command: command.to_owned(), ask: Ask::Auto }
    }

    /// Whether running it could change something, so the assistant asks first.
    pub fn critical(&self) -> bool {
        match self.ask {
            Ask::Auto => looks_critical(&self.command),
            Ask::Always => true,
            Ask::Never => false,
        }
    }
}

/// Whether the assistant asks before running a command.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ask {
    /// As the command looks: critical unless it only reads.
    #[default]
    Auto,
    Always,
    Never,
}

impl Ask {
    fn word(self) -> &'static str {
        match self {
            Ask::Auto => "auto",
            Ask::Always => "ask",
            Ask::Never => "free",
        }
    }

    fn from_word(word: &str) -> Ask {
        match word {
            "ask" => Ask::Always,
            "free" => Ask::Never,
            _ => Ask::Auto,
        }
    }

    fn next(self) -> Ask {
        match self {
            Ask::Auto => Ask::Always,
            Ask::Always => Ask::Never,
            Ask::Never => Ask::Auto,
        }
    }
}

/// Programs that only look, whatever they are given.
const READ_ONLY: &[&str] = &[
    "ping", "df", "free", "top", "logread", "cat", "head", "tail", "grep", "egrep", "fgrep", "ls", "uptime", "uname", "echo", "printf", "wc", "sort", "uniq",
    "cut", "tr", "jq", "ps", "netstat", "ss", "nslookup", "dig", "host", "traceroute", "tracepath", "du", "stat", "md5sum", "sha256sum", "cmp", "diff",
    "which", "whoami", "id", "printenv", "cd", "pwd", "lsusb", "lspci", "lsblk", "lsmod", "i2cdetect", "i2cget", "i2cdump", "vmstat", "sensors", "awk", "true",
];

/// Whether a command could change something: anything not known to only
/// read is taken as able to (an unknown program, a script, ssh, git, a
/// redirect into a file).
pub fn looks_critical(command: &str) -> bool {
    // Redirects into a file write it; into the other stream or nowhere do not.
    let mut rest = command.to_owned();
    for harmless in ["2>&1", ">&2", "2>/dev/null", ">/dev/null", "2> /dev/null", "> /dev/null"] {
        rest = rest.replace(harmless, " ");
    }
    if rest.contains('>') || rest.contains("system(") {
        return true;
    }
    rest.split(['|', ';', '&', '\n']).map(str::trim).filter(|part| !part.is_empty()).any(|part| !reads_only(part))
}

/// Whether one command of a pipeline only reads.
fn reads_only(part: &str) -> bool {
    let mut words: Vec<&str> = part.split_whitespace().collect();
    // Variables set for it, and `timeout N` or `nice` in front, change nothing.
    while let Some(first) = words.first() {
        if first.contains('=') && !first.starts_with('-') {
            words.remove(0);
        } else if *first == "timeout" && words.len() > 1 {
            words.drain(..2);
        } else if *first == "nice" {
            words.remove(0);
        } else {
            break;
        }
    }
    let Some(program) = words.first().map(|w| w.rsplit('/').next().unwrap_or(w)) else { return true };
    let args = &words[1..];
    // A flag as given, as `--flag=value`, or a short one with its value or
    // other short flags run on (`-XPOST`, `-sd`).
    let has = |flags: &[&str]| {
        args.iter().any(|a| {
            flags.iter().any(|f| {
                *a == *f
                    || a.strip_prefix(f).is_some_and(|after| after.starts_with('='))
                    || (f.len() == 2 && !a.starts_with("--") && a.starts_with('-') && a[1..].contains(&f[1..]))
            })
        })
    };
    let says = |verbs: &[&str]| args.iter().any(|a| verbs.contains(a));
    match program {
        "sort" => !has(&["-o", "--output"]),
        "uniq" => args.iter().filter(|a| !a.starts_with('-')).count() < 2,
        "ss" => !has(&["-K", "--kill"]),
        "curl" => !has(&["-X", "--request", "-d", "--data", "--data-raw", "--data-binary", "--data-urlencode", "--json", "-F", "--form", "-T", "--upload-file", "-o", "--output", "-O", "--remote-name", "-K", "--config"]),
        "wget" => args.iter().any(|a| *a == "-O-" || a.ends_with("O-")) || args.windows(2).any(|w| w[0] == "-O" && w[1] == "-"),
        "ip" => !says(&["add", "del", "delete", "set", "flush", "change", "replace", "append", "up", "down"]),
        "iw" => !says(&["set", "connect", "disconnect", "add", "del"]),
        "dmesg" => !has(&["-c", "-C", "--clear", "--read-clear"]),
        "date" => !has(&["-s", "--set"]),
        "find" => !says(&["-delete", "-exec", "-execdir", "-ok", "-fprint", "-fprintf"]),
        "sed" => !args.iter().any(|a| a.starts_with("-i") || a.starts_with("--in-place")),
        "hostname" => args.iter().all(|a| a.starts_with('-')),
        other => READ_ONLY.contains(&other),
    }
}

/// The built-in Actions: the robot, the network, the handheld. Each says
/// whether the assistant may run it freely (`Auto`: as the command reads) or
/// must ask (`Always`); `Never` for those that only look but whose shell
/// form the guess cannot read.
const BUILT_IN: [(&str, &str, Ask); 26] = [
    // The robot.
    ("Ping the robot", "ping -c 4 {robot}", Ask::Auto),
    ("Ask the robot's agent", "curl -fsS http://{robot}:9010/api/system", Ask::Auto),
    ("Logs on the robot", "curl -fsS http://{robot}:9010/api/logs", Ask::Auto),
    ("Trace to the robot", "traceroute -n -w 2 -m 8 {robot}", Ask::Auto),
    ("Robot's name and address", "nslookup {robot}", Ask::Auto),
    // The network.
    ("Wi-Fi link", "iw dev wlan0 link", Ask::Auto),
    ("Wi-Fi networks nearby", "iw dev wlan0 scan | grep -e SSID -e signal", Ask::Auto),
    ("Ethernet link", "ip link show eth0", Ask::Auto),
    ("Addresses", "ip addr", Ask::Auto),
    ("Routes", "ip route", Ask::Auto),
    ("Name servers", "cat /etc/resolv.conf", Ask::Auto),
    ("Internet reachable", "ping -c 3 -W 2 1.1.1.1", Ask::Auto),
    // The handheld.
    ("Disk space", "df -h", Ask::Auto),
    ("Memory", "free -m", Ask::Auto),
    ("Busiest processes", "top -b -n 1 | head -n 20", Ask::Auto),
    ("Uptime and load", "uptime", Ask::Auto),
    ("Clock", "date", Ask::Auto),
    ("Processor temperature", "cat /sys/class/thermal/thermal_zone0/temp", Ask::Auto),
    ("Processor speed", "cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq", Ask::Auto),
    ("Services", "rc-status", Ask::Never),
    ("USB devices", "lsusb", Ask::Auto),
    ("I2C devices", "i2cdetect -y -r 1", Ask::Auto),
    ("Kernel messages", "dmesg | tail -n 40", Ask::Auto),
    ("System log", "logread | tail -n 40", Ask::Auto),
    // Changes: always asked first.
    ("Restart Wi-Fi", "rc-service catalyst-wifi restart", Ask::Always),
    ("Restart the handheld", "reboot", Ask::Always),
];

/// The built-in Actions.
pub fn builtin() -> Vec<Saved> {
    BUILT_IN.iter().map(|(name, command, ask)| Saved { name: (*name).into(), command: (*command).into(), ask: *ask }).collect()
}

/// Every Action: the team's own, then the built-in ones they have not
/// replaced with their own of the same name.
pub fn all() -> Vec<Saved> {
    let mut all = load();
    all.extend(builtin().into_iter().filter(|b| !all.iter().any(|s| s.name.eq_ignore_ascii_case(&b.name))).collect::<Vec<_>>());
    all
}

fn file() -> PathBuf {
    crate::config_dir().join("commands")
}

/// Which commands the assistant was told otherwise about, one a line as
/// `name<tab>ask|free`. Kept apart from the commands so an older shell (after
/// an update is rolled back) still reads its list as it always did.
fn asks_file() -> PathBuf {
    crate::config_dir().join("commands-ask")
}

/// The team's own commands, one a line as `name<tab>command`. (Before 0.4.1
/// a list never changed held a few ready ones; they are built in now, under
/// the same names.)
pub fn load() -> Vec<Saved> {
    let mut saved: Vec<Saved> = match std::fs::read_to_string(file()) {
        Ok(text) => text
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .filter(|(n, c)| !n.trim().is_empty() && !c.trim().is_empty())
            .map(|(n, c)| Saved::new(n.trim(), c.trim()))
            .collect(),
        // None of their own yet: the built-in ones are there all the same.
        Err(_) => Vec::new(),
    };
    if let Ok(text) = std::fs::read_to_string(asks_file()) {
        for (name, word) in text.lines().filter_map(|l| l.split_once('\t')) {
            if let Some(s) = saved.iter_mut().find(|s| s.name == name.trim()) {
                s.ask = Ask::from_word(word.trim());
            }
        }
    }
    saved
}

pub fn save(saved: &[Saved]) -> bool {
    let text: String = saved.iter().map(|s| format!("{}\t{}\n", s.name, s.command)).collect();
    let asks: String = saved.iter().filter(|s| s.ask != Ask::Auto).map(|s| format!("{}\t{}\n", s.name, s.ask.word())).collect();
    std::fs::create_dir_all(crate::config_dir()).ok();
    crate::sys::write_atomic(&file(), text.as_bytes()).is_ok() && crate::sys::write_atomic(&asks_file(), asks.as_bytes()).is_ok()
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

/// The team's own commands, a row to add one, then the built-in ones.
pub struct Commands {
    saved: Vec<Saved>,
    builtin: Vec<Saved>,
    list: ListState,
    deleting: Option<usize>,
}

impl Commands {
    pub fn new() -> Self {
        Commands { saved: load(), builtin: shown_builtin(&load()), list: ListState::default(), deleting: None }
    }

    fn len(&self) -> usize {
        self.saved.len() + 1 + self.builtin.len()
    }

    /// The command on row `i`: the team's, or a built-in one (and which).
    fn at(&self, i: usize) -> Option<(&Saved, bool)> {
        match i.checked_sub(self.saved.len() + 1) {
            Some(b) => self.builtin.get(b).map(|s| (s, true)),
            None => self.saved.get(i).map(|s| (s, false)),
        }
    }
}

/// The built-in commands not replaced by one of the team's of the same name.
fn shown_builtin(saved: &[Saved]) -> Vec<Saved> {
    builtin().into_iter().filter(|b| !saved.iter().any(|s| s.name.eq_ignore_ascii_case(&b.name))).collect()
}

impl Screen for Commands {
    fn title(&self) -> &str {
        "Actions"
    }

    fn resume(&mut self) {
        self.saved = load();
        self.builtin = shown_builtin(&self.saved);
        self.list.focus = self.list.focus.min(self.len() - 1);
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        let armed = self.deleting.take();
        let at = self.list.focus;
        match key {
            Key::Back => return Nav::Pop,
            Key::Enter | Key::Right if at == self.saved.len() => return Nav::Push(Box::new(Edit::new(None, None))),
            Key::Enter | Key::Right => {
                let Some((s, _)) = self.at(at) else { return Nav::Stay };
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
            // A built-in one stays as it is: a copy of it becomes the team's own.
            Key::Char('e') if at > self.saved.len() => {
                if let Some((s, _)) = self.at(at) {
                    return Nav::Push(Box::new(Edit::new(None, Some(s.clone()))));
                }
            }
            Key::Char('d') if at > self.saved.len() => *cx.say = Some("built in: it stays · e keeps a copy to change".into()),
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
            other => {
                let len = self.len();
                crate::screens::list_key(&mut self.list, other, len);
            }
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        let len = self.len();
        for (i, y) in self.list.layout(g, len, top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            match self.at(i) {
                Some((s, built_in)) => {
                    // As much of the command as fits beside its name.
                    let style = if g.theme == crate::look::Theme::Bezel { g.ty.body } else { g.ty.label };
                    let name_w = g.measure(&s.name, g.ty.body).max(60.0);
                    let preview = g.text.fit(&s.command, &style, (r.w - name_w - 34.0).max(30.0));
                    g.row(r, &s.name, &preview, if built_in { t.faint } else { t.dim }, i == self.list.focus);
                }
                None => g.row(r, "New command", "n", t.dim, i == self.list.focus),
            }
        }
        self.list.scrollbar(g, len, top, bottom);
        let focus = self.list.focus;
        g.hint(match focus.cmp(&self.saved.len()) {
            std::cmp::Ordering::Less => "enter: run · e: edit · d: remove",
            std::cmp::Ordering::Equal => "enter: a new one",
            std::cmp::Ordering::Greater => "built in · enter: run · e: a copy to change",
        });
    }
}

// ------------------------------------------------------------------ edit ---

/// A command's name and line, typed one after the other, and whether the
/// assistant asks before running it.
pub struct Edit {
    /// Which command this changes, or none for a new one.
    index: Option<usize>,
    name: String,
    command: String,
    ask: Ask,
    list: ListState,
    field: Option<(Field, bool)>,
}

/// Name, command, the assistant, keep it.
const EDIT_ROWS: usize = 4;

impl Edit {
    fn new(index: Option<usize>, saved: Option<Saved>) -> Self {
        let (name, command, ask) = saved.map_or((String::new(), String::new(), Ask::Auto), |s| (s.name, s.command, s.ask));
        Edit { index, name, command, ask, list: ListState::default(), field: None }
    }

    /// What the assistant does with it, as the row says.
    fn ask_words(&self) -> &'static str {
        let critical = Saved { name: String::new(), command: self.command.clone(), ask: self.ask }.critical();
        match (self.ask, critical) {
            (Ask::Auto, true) => "asks first · auto",
            (Ask::Auto, false) => "runs it · auto",
            (_, true) => "asks first",
            (_, false) => "runs it",
        }
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
                    self.list.focus = if self.command.is_empty() { 1 } else { EDIT_ROWS - 1 };
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
            (Key::Enter | Key::Right, 2) => self.ask = self.ask.next(),
            (Key::Enter, _) => {
                if self.name.is_empty() || self.command.is_empty() {
                    *cx.say = Some("it needs a name and a command".into());
                    self.list.focus = if self.name.is_empty() { 0 } else { 1 };
                    return Nav::Stay;
                }
                let mut saved = load();
                let this = Saved { name: self.name.clone(), command: self.command.clone(), ask: self.ask };
                match self.index.filter(|i| *i < saved.len()) {
                    Some(i) => saved[i] = this,
                    None => saved.push(this),
                }
                *cx.say = Some(if save(&saved) { format!("kept {}", self.name.to_lowercase()) } else { "could not keep it".into() });
                return Nav::Pop;
            }
            (other, _) => crate::screens::list_key(&mut self.list, other, EDIT_ROWS),
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
        for (i, y) in self.list.layout(g, EDIT_ROWS, top, row_h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let (name, value) = match i {
                0 => ("Name", if self.name.is_empty() { "–".to_owned() } else { self.name.clone() }),
                1 => ("Command", if self.command.is_empty() { "–".to_owned() } else { g.text.fit(&self.command, &style, r.w - 110.0) }),
                2 => ("Assistant", self.ask_words().to_owned()),
                _ => ("Keep it", "›".to_owned()),
            };
            g.row(r, name, &value, if i == EDIT_ROWS - 1 { t.ink } else { t.dim }, i == self.list.focus);
        }
        self.list.scrollbar(g, EDIT_ROWS, top, bottom);
        g.hint(match self.list.focus {
            2 => "enter: auto · asks first · runs it",
            _ => "{robot} and {team} are filled in when it runs",
        });
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
    fn the_teams_commands_are_kept_and_the_built_in_ones_follow() {
        crate::shell::tests::sandbox();
        std::fs::remove_file(file()).ok();
        assert!(load().is_empty(), "none of their own to begin with");
        assert_eq!(all(), builtin(), "but the built-in ones all the same");
        assert!(builtin().iter().any(|s| s.command.contains("{robot}")));
        let mut mine = vec![Saved::new("Deploy", "cd /data/code/robot && ./gradlew deploy"), Saved::new("Ping the robot", "ping -c 20 {robot}")];
        mine[1].ask = Ask::Never;
        assert!(save(&mine));
        assert_eq!(load(), mine, "what the assistant was told about each is kept too");
        // The list itself stays in the form an older shell reads.
        assert_eq!(std::fs::read_to_string(file()).unwrap().lines().next(), Some("Deploy\tcd /data/code/robot && ./gradlew deploy"));
        // Theirs first; a built-in one of the same name gives way to theirs.
        let every = all();
        assert_eq!(&every[..2], &mine[..]);
        assert_eq!(every.iter().filter(|s| s.name == "Ping the robot").count(), 1);
        assert_eq!(every.len(), 2 + builtin().len() - 1);
        assert!(save(&[]));
        assert!(load().is_empty());
        std::fs::remove_file(file()).ok();
        std::fs::remove_file(asks_file()).ok();
    }

    #[test]
    fn the_built_in_commands_say_rightly_whether_they_change_anything() {
        let names: Vec<&str> = BUILT_IN.iter().map(|(n, ..)| *n).collect();
        assert_eq!(names.len(), names.iter().collect::<std::collections::HashSet<_>>().len(), "no two share a name");
        for (name, command, ask) in BUILT_IN {
            match ask {
                Ask::Auto => assert!(!looks_critical(command), "{name}: {command} reads as a change"),
                Ask::Always => assert!(looks_critical(command), "{name}: {command} is asked about, and reads as a change too"),
                Ask::Never => {}
            }
        }
        assert!(builtin().iter().filter(|s| s.critical()).all(|s| s.name.starts_with("Restart")), "only the restarts ask first");
        for command in [
            "cd /data/code/robot && ./gradlew deploy",
            "ssh admin@{robot} sudo reboot",
            "curl -X POST http://{robot}:9010/api/restart",
            "curl -s -d x=1 http://{robot}/",
            "curl --data=x http://{robot}/",
            "curl -sO http://example.com/file",
            "df -h > /data/space.txt",
            "rm -rf /data/logs",
            "ip link set eth0 down",
            "dmesg -c",
            "git push",
            "find /data -name '*.tmp' -delete",
            "sed -i s/a/b/ /etc/config",
            "awk 'BEGIN { system(\"reboot\") }'",
            "env X=1 reboot",
            "sort -o /etc/hosts /etc/hosts",
            "wget http://example.com/file",
        ] {
            assert!(looks_critical(command), "{command}");
        }
        for command in ["logread | tail -n 40 2>&1", "curl -fsS http://{robot}:9010/api/system", "ip route", "dmesg -T | grep usb", "LANG=C df -h", "timeout 5 ping {robot}", "wget -qO- http://{robot}/"] {
            assert!(!looks_critical(command), "{command}");
        }
        // Told otherwise, either way.
        let mut deploy = Saved::new("Deploy", "./gradlew deploy");
        assert!(deploy.critical());
        deploy.ask = Ask::Never;
        assert!(!deploy.critical());
        let mut space = Saved::new("Disk", "df -h");
        space.ask = Ask::Always;
        assert!(space.critical());
    }

    #[test]
    fn a_command_is_typed_without_tabs_or_line_breaks() {
        assert!(command_char('|') && command_char('{') && command_char(' '));
        assert!(!command_char('\t') && !command_char('\n'));
    }
}
