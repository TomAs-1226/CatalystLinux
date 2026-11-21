//! `ai` in the Terminal: the assistant, for a shell.
//!
//!   ai <question>               an answer in plain words
//!   ai cmd <what you want>      one command proposed; it runs only on a yes
//!   ai app <what it should do>  an app written for you (a script on the
//!                               handheld's own screens, catalyst-ui), put
//!                               on Home only on a yes
//!   <command> 2>&1 | ai explain [question]
//!                               what some output or an error means
//!   ai --local …                the on-device model rather than a key
//!   ai --listen [cmd]           the question (or what the command should
//!                               do) said aloud: the microphone listens until
//!                               Enter, and OpenAI's gpt-transcribe writes it
//!                               down with the OpenAI key
//!
//! It answers with the service the Assistant app uses (Settings › Keys:
//! OpenAI's GPT-6.2 Sol or Claude Opus 5.5, at the effort chosen there), or
//! with the on-device model when asked to or when no key is set (not for
//! `ai cmd`: see NO_LOCAL_COMMANDS). Keys reach curl through a file only this
//! user can read, never on its command line.
//! Nothing is ever run without a "y" typed at the question (and "yes" for a
//! command that could delete or change things).

use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use localmodel as lm;
use serde_json::{json, Value};

const OPENAI_URL: &str = "https://api.openai.com/v1/responses";
const CLAUDE_URL: &str = "https://api.anthropic.com/v1/messages";
const CLAUDE_VERSION: &str = "2026-10-21";
const CLAUDE_BETA: &str = "server-side-fallback-2026-07-01";
const CA_BUNDLE: &str = "/etc/ssl/certs/ca-certificates.crt";
/// What of piped output is sent: its end, where the error usually is.
const EXPLAIN_KEEP: usize = 12_000;

const ABOUT: &str = "You answer in a terminal on Catalyst Linux, a pocket Linux handheld (an M5Stack CardputerZero) that an FRC robotics team carries. It runs BusyBox (ash and its small tools), musl and OpenRC as root; /data keeps files across restarts and the system itself is read-only.";
const PLAIN: &str = "Write plain text for a narrow terminal: no Markdown, no headings, no tables; short paragraphs; under about 120 words unless asked for more.";
const CMD_RULES: &str = "Propose exactly one shell command, for BusyBox ash on this handheld, that does what is asked. Reply in exactly two lines:\nCOMMAND: <the command, on one line>\nWHY: <one short sentence on what it does>\nIf one command cannot do it safely, reply \"COMMAND: none\" and give the reason as the WHY.";
const EXPLAIN_RULES: &str = "Explain what the output below means and, if something is wrong, what to do about it, briefly.";

const HELP: &str = "ai: the assistant, in the terminal

  ai <question>
      an answer in plain words
  ai cmd <what you want>
      one command, run on your yes (needs a key)
  ai app <what it should do>
      an app for Home, written for you (needs a key)
  <command> 2>&1 | ai explain [question]
      what some output or an error means

  --local   the on-device model
            (Settings > On-device model)
  --cloud   use a key even with the model here
  --listen  say it aloud; enter stops listening
            (ai --listen, ai --listen cmd)

examples
  ai what does a brownout do to the robot
  ai cmd find the ten biggest files in /data
  ai app a log of pit batteries and their volts
  dmesg | tail -n 30 | ai explain";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Service {
    OpenAi,
    Claude,
    Local,
}

#[derive(Debug, PartialEq)]
enum Mode {
    Ask(String),
    Cmd(String),
    Explain(String),
    /// An app written for you: a script using catalyst-ui, put on Home on a yes.
    App(String),
    Help,
}

#[derive(Debug, PartialEq)]
struct Args {
    mode: Mode,
    local: Option<bool>,
    /// The question (or the command's purpose) said aloud.
    listen: bool,
}

fn parse_args(args: &[String]) -> Args {
    let mut local = None;
    let mut listen = false;
    let mut words: Vec<&str> = Vec::new();
    for a in args {
        match a.as_str() {
            "--local" => local = Some(true),
            "--cloud" => local = Some(false),
            "--listen" | "-l" => listen = true,
            "-h" | "--help" | "help" if words.is_empty() => return Args { mode: Mode::Help, local, listen: false },
            other => words.push(other),
        }
    }
    let rest = |from: usize| words[from..].join(" ");
    let mode = match words.first().copied() {
        // Nothing typed, to be said instead.
        None if listen => Mode::Ask(String::new()),
        None => Mode::Help,
        Some("cmd") => Mode::Cmd(rest(1)),
        Some("explain") => Mode::Explain(rest(1)),
        Some("app") => Mode::App(rest(1)),
        Some(_) => Mode::Ask(rest(0)),
    };
    Args { mode, local, listen }
}

// ------------------------------------------------------------------ setup ---

/// The handheld's own folder (keys, the assistant's choices): beside the
/// robot address the shell is told of (`CATALYST_CONF`), else /data/catalyst.
fn config_dir() -> PathBuf {
    if let Some(conf) = std::env::var_os("CATALYST_CONF") {
        if let Some(parent) = Path::new(&conf).parent() {
            return parent.to_path_buf();
        }
    }
    PathBuf::from("/data/catalyst")
}

/// A `name=value` file as pairs.
fn pairs(text: &str) -> Vec<(String, String)> {
    text.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned())).filter(|(_, v)| !v.is_empty()).collect()
}

/// The key for a service, from the keys file (0.4.0's single "ai" key counts
/// for whichever service it belongs to).
fn key_for(keys: &[(String, String)], service: Service) -> Option<String> {
    let name = match service {
        Service::OpenAi => "openai",
        Service::Claude => "claude",
        Service::Local => return None,
    };
    keys.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()).or_else(|| {
        let old = keys.iter().find(|(k, _)| k == "ai").map(|(_, v)| v.clone())?;
        (old.starts_with("sk-ant-") == (service == Service::Claude)).then_some(old)
    })
}

/// Who answers: the on-device model when asked for (or when there is no key
/// and it is here), else the service chosen in the Assistant, else the first
/// with a key.
fn choose(local: Option<bool>, prefs: &[(String, String)], keys: &[(String, String)], model_here: bool) -> Result<(Service, Option<String>), String> {
    if local == Some(true) {
        return if model_here { Ok((Service::Local, None)) } else { Err("the on-device model is not downloaded: Settings > On-device model".into()) };
    }
    let chosen = prefs.iter().find(|(k, _)| k == "service").and_then(|(_, v)| match v.as_str() {
        "openai" => Some(Service::OpenAi),
        "claude" => Some(Service::Claude),
        _ => None,
    });
    for service in chosen.into_iter().chain([Service::OpenAi, Service::Claude]) {
        if let Some(key) = key_for(keys, service) {
            return Ok((service, Some(key)));
        }
    }
    if model_here && local != Some(false) {
        return Ok((Service::Local, None));
    }
    Err("no key yet: add an OpenAI or Claude key in Settings > Keys, or download the on-device model (Settings > On-device model)".into())
}

/// "low" or "medium", as the Assistant was set.
fn effort(prefs: &[(String, String)]) -> &'static str {
    match prefs.iter().find(|(k, _)| k == "effort").map(|(_, v)| v.as_str()) {
        Some("light") => "low",
        _ => "medium",
    }
}

// --------------------------------------------------------------- requests ---

fn openai_body(system: &str, user: &str, effort: &str) -> Value {
    json!({
        "model": "gpt-6.2-sol",
        "instructions": system,
        "input": [{"role": "user", "content": user}],
        "reasoning": {"effort": effort},
        "store": false,
        "max_output_tokens": 8000,
    })
}

fn claude_body(system: &str, user: &str, effort: &str) -> Value {
    json!({
        "model": "claude-opus-5-5",
        "max_tokens": 4000,
        "system": system,
        "messages": [{"role": "user", "content": user}],
        "output_config": {"effort": effort},
        "fallbacks": "default",
    })
}

/// The words of an answer, or why there are none.
fn answer_of(service: Service, v: &Value) -> Result<String, String> {
    if let Some(message) = v["error"]["message"].as_str() {
        return Err(message.to_owned());
    }
    let mut text = String::new();
    match service {
        Service::OpenAi => {
            for item in v["output"].as_array().into_iter().flatten() {
                for part in item["content"].as_array().into_iter().flatten() {
                    match part["type"].as_str() {
                        Some("output_text") => text.push_str(part["text"].as_str().unwrap_or_default()),
                        Some("refusal") => return Err(format!("it would not answer: {}", part["refusal"].as_str().unwrap_or("no reason given"))),
                        _ => {}
                    }
                }
            }
        }
        Service::Claude => {
            if v["stop_reason"] == "refusal" {
                return Err(format!("it would not answer: {}", v["stop_details"]["explanation"].as_str().unwrap_or("no reason given")));
            }
            for block in v["content"].as_array().into_iter().flatten() {
                if block["type"] == "text" {
                    text.push_str(block["text"].as_str().unwrap_or_default());
                }
            }
        }
        Service::Local => text = lm::chat_answer(v)?,
    }
    if text.trim().is_empty() {
        Err("an empty answer came back".into())
    } else {
        Ok(text.trim().to_owned())
    }
}

/// One request to a cloud service with curl, its key in a file only this
/// user reads (removed straight after).
fn post(service: Service, key: &str, body: &Value) -> Result<Value, String> {
    let (url, headers) = match service {
        Service::OpenAi => (OPENAI_URL, format!("Authorization: Bearer {key}\nContent-Type: application/json\n")),
        Service::Claude => (CLAUDE_URL, format!("x-api-key: {key}\nanthropic-version: {CLAUDE_VERSION}\nanthropic-beta: {CLAUDE_BETA}\nContent-Type: application/json\n")),
        Service::Local => return Err("not a cloud service".into()),
    };
    let file = private_file(&headers)?;
    let header_arg = format!("@{}", file.display());
    let result = (|| {
        let mut command = Command::new("curl");
        command.args(["-sS", "--max-time", "180", "--proto", "=https", "--data-binary", "@-", "-w", "\n%{http_code}", "-H", &header_arg]);
        if Path::new(CA_BUNDLE).exists() {
            command.args(["--cacert", CA_BUNDLE]);
        }
        let mut child = command.arg(url).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("could not run curl: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(body.to_string().as_bytes()).map_err(|e| format!("sending the question: {e}"))?;
        }
        child.wait_with_output().map_err(|e| format!("the request: {e}"))
    })();
    std::fs::remove_file(&file).ok();
    let out = result?;
    if !out.status.success() {
        return Err(match out.status.code() {
            Some(6) => "no internet: the service's name did not resolve".into(),
            Some(7) => "could not reach the service".into(),
            Some(28) => "no answer in time".into(),
            Some(35 | 60) => "the secure connection failed: is the clock right?".into(),
            _ => format!("the request failed: {}", String::from_utf8_lossy(&out.stderr).trim()),
        });
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    let code: u16 = code.trim().parse().unwrap_or(0);
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    match code {
        200 => Ok(v),
        401 | 403 => Err("the key was refused: check it in Settings > Keys".into()),
        402 | 429 => Err("too many requests, or the account is out of credit".into()),
        _ => Err(v["error"]["message"].as_str().map_or_else(|| format!("the service answered {code}"), str::to_owned)),
    }
}

/// A file only this user can read, holding `text`, in a private folder.
fn private_file(text: &str) -> Result<PathBuf, String> {
    let path = std::env::temp_dir().join(format!("ai-{}.headers", std::process::id()));
    std::fs::remove_file(&path).ok();
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(|e| format!("preparing the request: {e}"))?;
    file.write_all(text.as_bytes()).map_err(|e| format!("preparing the request: {e}"))?;
    Ok(path)
}

/// The small on-device model runs on well past what it is asked for, so it
/// is held to a few sentences and a token limit (`tokens`) as well.
const LOCAL_BRIEF: &str = "Answer in at most three short sentences.";

/// Ask whoever answers, a system prompt and one question.
fn ask(service: Service, key: Option<&str>, system: &str, user: &str, effort: &str, tokens: u32) -> Result<String, String> {
    match service {
        Service::Local => ask_local(&format!("{system}\n\n{LOCAL_BRIEF}"), user, tokens),
        cloud => {
            let body = if cloud == Service::OpenAi { openai_body(system, user, effort) } else { claude_body(system, user, effort) };
            answer_of(cloud, &post(cloud, key.unwrap_or_default(), &body)?)
        }
    }
}

fn ask_local(system: &str, user: &str, tokens: u32) -> Result<String, String> {
    ask_local_body(&lm::chat_body(system, &[("user", user)], tokens))
}

/// Why `ai cmd` does not use the on-device model. Tried end to end with the
/// 135M held to a JSON shape: the shape came back every time, the commands
/// were made up ("print_current_date", "busybox --free"). Proposing commands
/// that cannot work is worse than saying so.
const NO_LOCAL_COMMANDS: &str = "the on-device model is too small to write commands you can trust: ai cmd needs an OpenAI or Claude key (Settings > Keys)";

/// The on-device model: the one already running (the shell's), else one
/// started for this question and stopped after it.
fn ask_local_body(body: &Value) -> Result<String, String> {
    let body = body.clone();
    if lm::healthy() {
        return lm::ask(&body, Duration::from_secs(300)).map(|(a, _)| a);
    }
    let dir = lm::dir();
    let mut child = lm::spawn_server(&dir, lm::chosen(&dir))?;
    let answer = lm::wait_ready(&mut child, Duration::from_secs(120)).and_then(|()| lm::ask(&body, Duration::from_secs(300)).map(|(a, _)| a));
    lm::stop_server(&mut child);
    answer
}

// ---------------------------------------------------------------- answers ---

/// Markdown's marks taken out, as the Assistant app does.
fn plain(text: &str) -> String {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim_start().starts_with("```") {
            continue;
        }
        let line = match line.trim_start_matches('#') {
            heading if line.starts_with('#') && heading.starts_with(' ') => heading.trim_start(),
            _ => line,
        };
        let line = match line.trim_start().strip_prefix("- ").or_else(|| line.trim_start().strip_prefix("* ")) {
            Some(item) => format!("· {item}"),
            None => line.to_owned(),
        };
        out.push(line.replace("**", "").replace("__", ""));
    }
    out.join("\n").trim().to_owned()
}

/// Text broken into lines no wider than `width`, keeping blank lines.
fn wrap(text: &str, width: usize) -> String {
    let width = width.max(20);
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        out.push(line);
    }
    out.join("\n")
}

/// The terminal's width: what it says, else COLUMNS, else the handheld's 53.
fn columns() -> usize {
    #[cfg(unix)]
    {
        let mut size: libc::winsize = unsafe { std::mem::zeroed() };
        // SAFETY: asking the terminal on stdout its size.
        if unsafe { libc::ioctl(1, libc::TIOCGWINSZ, &mut size) } == 0 && size.ws_col > 0 {
            return size.ws_col as usize;
        }
    }
    std::env::var("COLUMNS").ok().and_then(|c| c.parse().ok()).unwrap_or(53)
}

// ---------------------------------------------------------------- commands ---

/// The command a reply proposes and why, or why it proposes none.
fn proposal(reply: &str) -> Result<(String, String), String> {
    let clean = |s: &str| s.trim().trim_matches('*').trim().trim_matches('`').trim().to_owned();
    let mut command = None;
    let mut why = String::new();
    for line in reply.lines() {
        let line = line.trim().trim_start_matches(['*', '-', ' ']);
        let upper = line.to_ascii_uppercase();
        if upper.starts_with("COMMAND:") {
            command = Some(clean(&line["COMMAND:".len()..]));
        } else if upper.starts_with("WHY:") {
            why = clean(&line["WHY:".len()..]);
        }
    }
    // No COMMAND line: a lone fenced block, at least.
    if command.is_none() {
        let fenced: Vec<&str> = reply.lines().skip_while(|l| !l.trim_start().starts_with("```")).skip(1).take_while(|l| !l.trim_start().starts_with("```")).collect();
        if fenced.len() == 1 {
            command = Some(clean(fenced[0]));
        }
    }
    match command {
        Some(c) if c.eq_ignore_ascii_case("none") || c.is_empty() => Err(if why.is_empty() { "it proposed no command".into() } else { why }),
        Some(c) if c.contains('\n') => Err("it proposed more than one line".into()),
        Some(c) => Ok((c, why)),
        None => Err("it did not propose a command".into()),
    }
}

/// Whether a command could delete or change things, so a plain "y" is not
/// enough: anything that removes, overwrites, formats, kills, restarts or
/// writes to a file.
fn dangerous(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let mut rest = lower.clone();
    for harmless in ["2>&1", ">&2", "2>/dev/null", ">/dev/null", "2> /dev/null", "> /dev/null"] {
        rest = rest.replace(harmless, " ");
    }
    if rest.contains('>') {
        return true;
    }
    const PROGRAMS: [&str; 24] = [
        "rm", "rmdir", "dd", "mkfs", "mke2fs", "wipefs", "fdisk", "sfdisk", "parted", "shred", "truncate", "mv", "reboot", "poweroff", "halt", "shutdown",
        "kill", "killall", "pkill", "tee", "chmod", "chown", "rc-service", "mount",
    ];
    let words: Vec<&str> = lower.split(|c: char| c.is_whitespace() || matches!(c, '|' | ';' | '&' | '(' | ')')).filter(|w| !w.is_empty()).collect();
    let program = |w: &str| w.rsplit('/').next().unwrap_or(w).to_owned();
    words.iter().any(|w| PROGRAMS.contains(&program(w).as_str()) || program(w).starts_with("mkfs."))
        || words.windows(2).any(|w| (program(w[0]) == "sed" && w[1].starts_with("-i")) || (program(w[0]) == "git" && matches!(w[1], "push" | "reset" | "clean" | "checkout")))
        || words.iter().any(|w| matches!(*w, "-delete" | "-exec" | "-execdir"))
}

/// A question answered from the terminal itself, even with stdin piped:
/// whether the answer was one of `want`.
fn confirm(prompt: &str, want: &[&str]) -> bool {
    eprint!("{prompt}");
    std::io::stderr().flush().ok();
    let mut answer = String::new();
    let read = match std::fs::File::open("/dev/tty") {
        Ok(mut tty) => {
            let mut buf = [0u8; 64];
            tty.read(&mut buf).map(|n| answer = String::from_utf8_lossy(&buf[..n]).into_owned())
        }
        Err(_) => std::io::stdin().read_line(&mut answer).map(|_| ()),
    };
    read.is_ok() && want.iter().any(|w| answer.trim().eq_ignore_ascii_case(w))
}

// -------------------------------------------------------------------- apps ---

/// How `ai app` asks for an app: a script on the handheld's own screens.
const APP_RULES: &str = "Write a small app for this handheld: a BusyBox ash script that asks and answers with catalyst-ui, the handheld's own screens (320 by 170, keys only). Reply with exactly two files, each as a line `FILE: <name>` followed by one fenced block: first `app`, then `run.sh`. Nothing else.

The `app` file is `key = value` lines:
id = lowercase letters, digits and dashes
name = at most 24 characters
icon = one of: apps timer schedule checklist calculate notes sticky_note_2 battery_full bolt star folder event insights speed lightbulb network_check settings front_hand person emoji_events play_arrow history thermostat
about = at most 40 characters, lowercase
version = 1.0.0
run = sh ./run.sh
screen = own

catalyst-ui pages (each prints its answer on stdout; exit status 1 means Esc was pressed, so `|| exit 0` or `|| continue`):
  catalyst-ui menu TITLE ITEM...          prints the item chosen; an item may be icon:Label with an icon from the list above
  catalyst-ui input TITLE [PROMPT] [--value TEXT]   prints what was typed
  catalyst-ui confirm TITLE QUESTION [--yes WORD] [--no WORD]   exit status 0 for yes
  catalyst-ui message TITLE TEXT          until Enter or Esc
  catalyst-ui view TITLE [FILE]           a file, or text piped in, to scroll through

The script loops on a menu and ends when the menu is left (Esc). What it keeps goes in files under \"$CATALYST_APP_DATA\" (the app's own folder, kept across updates). Short titles; words a few lines long at most. It must never delete or change anything outside \"$CATALYST_APP_DATA\", and use the network only if asked to.";

/// The `app` file and `run.sh` an `ai app` reply holds.
fn app_files(reply: &str) -> Result<(String, String), String> {
    let mut files: Vec<(String, String)> = Vec::new();
    let mut lines = reply.lines();
    while let Some(line) = lines.next() {
        let Some(name) = line.trim().trim_matches(['*', '`']).strip_prefix("FILE:") else { continue };
        let name = name.trim().trim_matches(['*', '`']).trim().to_owned();
        // Its block: from the next fence to the one closing it.
        let mut body = Vec::new();
        let mut open = false;
        for l in lines.by_ref() {
            if l.trim_start().starts_with("```") {
                if open {
                    break;
                }
                open = true;
            } else if open {
                body.push(l);
            }
        }
        files.push((name, body.join("\n") + "\n"));
    }
    let find = |n: &str| files.iter().find(|(name, _)| name == n).map(|(_, b)| b.clone());
    match (find("app"), find("run.sh")) {
        (Some(app), Some(script)) => Ok((app, script)),
        _ => Err("it did not write an app file and a run.sh".into()),
    }
}

/// An app file fit to put on Home: its id, and the file as it will be
/// written (what the handheld needs, in its words). Each problem said plainly.
fn checked_app(app: &str) -> Result<(String, String), String> {
    let field = |key: &str| app.lines().filter_map(|l| l.split_once('=')).find(|(k, _)| k.trim() == key).map(|(_, v)| v.trim().to_owned()).unwrap_or_default();
    let id = field("id");
    if id.is_empty() || id.len() > 40 || id.starts_with('-') || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err(format!("its id ({id:?}) is not one a folder can take"));
    }
    let name: String = field("name").chars().take(24).collect();
    if name.is_empty() {
        return Err("it has no name".into());
    }
    if field("run") != "sh ./run.sh" || field("screen") != "own" {
        return Err("its app file does not run its script on the screen".into());
    }
    let about: String = field("about").chars().take(40).collect();
    let icon = field("icon");
    let file = format!(
        "id = {id}\nname = {name}\nicon = {}\nabout = {}\nversion = 1.0.0\nrun = sh ./run.sh\nscreen = own\ncredit = written by the assistant\n",
        if icon.is_empty() { "apps" } else { &icon },
        if about.is_empty() { "an app of my own" } else { &about }
    );
    Ok((id, file))
}

/// The folder apps of your own are kept in: the last of `CATALYST_APPS`
/// (as the shell reads it), else /data/apps.
fn apps_dir() -> PathBuf {
    std::env::var_os("CATALYST_APPS").and_then(|list| std::env::split_paths(&list).last()).unwrap_or_else(|| PathBuf::from("/data/apps"))
}

/// Put an app on Home: its folder, its app file, its script. Never over
/// an app that is there already.
fn put_app(dir: &Path, id: &str, app: &str, script: &str) -> Result<PathBuf, String> {
    let folder = dir.join(id);
    if folder.exists() {
        return Err(format!("there is an app called {id} already: ask for it by another name"));
    }
    std::fs::create_dir_all(&folder).map_err(|e| format!("making {}: {e}", folder.display()))?;
    std::fs::write(folder.join("app"), app).map_err(|e| e.to_string())?;
    std::fs::write(folder.join("run.sh"), script).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(folder.join("run.sh"), std::fs::Permissions::from_mode(0o755)).ok();
    }
    Ok(folder)
}

// ------------------------------------------------------------------ voice ---

/// Listening stops by itself after this long.
const LISTEN_MOST: std::time::Duration = std::time::Duration::from_secs(30);

/// A level as a meter a terminal can show: `[||||||......]`.
fn meter(level: f32) -> String {
    let filled = (level.clamp(0.0, 1.0) * 12.0).round() as usize;
    format!("[{}{}]", "|".repeat(filled), ".".repeat(12 - filled))
}

/// What is said aloud, written down: the microphone (ALSA's `default`)
/// listens until Enter (or 30 s), with a meter on the terminal, and OpenAI's
/// gpt-transcribe writes it down with the OpenAI key. The microphone is
/// closed again the moment it is done with.
fn listen_aloud(keys: &[(String, String)]) -> Result<String, String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};
    let key = key_for(keys, Service::OpenAi).ok_or("--listen needs an OpenAI key (Settings > Keys): OpenAI writes down what you say")?;
    let procs = speech::Procs::default();
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<Result<Vec<i16>, String>>();
    let (mic_procs, mic_stop) = (procs.clone(), stop.clone());
    std::thread::spawn(move || {
        use speech::source::Source;
        let mut mic = match speech::source::Arecord::start("default", &mic_procs) {
            Ok(mic) => mic,
            Err(why) => {
                tx.send(Err(why)).ok();
                return;
            }
        };
        while !mic_stop.load(Ordering::Relaxed) {
            let mut chunk = Vec::new();
            match mic.read(&mut chunk) {
                Ok(more) => {
                    if tx.send(Ok(chunk)).is_err() || !more {
                        break;
                    }
                }
                Err(why) => {
                    tx.send(Err(why)).ok();
                    break;
                }
            }
        }
    });
    // Enter, from the terminal itself (stdin may be piped).
    let (enter_tx, enter_rx) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        let mut line = String::new();
        match std::fs::File::open("/dev/tty") {
            Ok(tty) => {
                std::io::BufRead::read_line(&mut std::io::BufReader::new(tty), &mut line).ok();
            }
            Err(_) => {
                std::io::stdin().read_line(&mut line).ok();
            }
        }
        enter_tx.send(()).ok();
    });
    let shown = std::io::stderr().is_terminal();
    let started = std::time::Instant::now();
    let mut samples: Vec<i16> = Vec::new();
    let mut level = 0.0;
    let mut failed = None;
    loop {
        while let Ok(heard) = rx.try_recv() {
            match heard {
                Ok(chunk) => {
                    level = speech::level(&chunk);
                    samples.extend(chunk);
                }
                Err(why) => failed = Some(why),
            }
        }
        if failed.is_some() || enter_rx.try_recv().is_ok() || started.elapsed() >= LISTEN_MOST {
            break;
        }
        if shown {
            eprint!("\r\x1b[Klistening {} enter stops", meter(level));
            std::io::stderr().flush().ok();
        }
        std::thread::sleep(std::time::Duration::from_millis(80));
    }
    // The last of it, then the microphone closed.
    std::thread::sleep(std::time::Duration::from_millis(150));
    stop.store(true, Ordering::Relaxed);
    procs.kill_all();
    while let Ok(Ok(chunk)) = rx.recv_timeout(std::time::Duration::from_millis(100)) {
        samples.extend(chunk);
    }
    if shown {
        eprint!("\r\x1b[K");
    }
    if let (Some(why), true) = (failed, samples.is_empty()) {
        return Err(why);
    }
    let clip = speech::Clip { samples };
    if clip.silent() {
        return Err("heard nothing: is the microphone on?".into());
    }
    let text = {
        let _w = Waiting::new("writing it down…");
        speech::openai::transcribe(&key, &clip.wav(), speech::openai::KEYWORDS, &std::env::temp_dir())?
    };
    if text.trim().is_empty() {
        return Err("did not catch that: try again".into());
    }
    eprintln!("you: {}", text.trim());
    Ok(text.trim().to_owned())
}

// -------------------------------------------------------------------- main ---

/// "thinking…" on the terminal while it waits, gone when the answer comes.
struct Waiting(bool);

impl Waiting {
    fn new(words: &str) -> Self {
        let shown = std::io::stderr().is_terminal();
        if shown {
            eprint!("{words}");
            std::io::stderr().flush().ok();
        }
        Waiting(shown)
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        if self.0 {
            eprint!("\r\x1b[K");
        }
    }
}

fn run() -> Result<i32, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut args = parse_args(&args);
    if args.mode == Mode::Help {
        println!("{HELP}");
        return Ok(0);
    }
    let dir = config_dir();
    let keys = pairs(&std::fs::read_to_string(dir.join("keys")).unwrap_or_default());
    // Said aloud: the question, or what the command should do.
    if args.listen {
        args.mode = match args.mode {
            Mode::Ask(_) => Mode::Ask(listen_aloud(&keys)?),
            Mode::Cmd(_) => Mode::Cmd(listen_aloud(&keys)?),
            _ => return Err("--listen asks a question (ai --listen) or for a command (ai --listen cmd)".into()),
        };
    }
    let prefs = pairs(&std::fs::read_to_string(dir.join("assistant")).unwrap_or_default());
    let models = lm::dir();
    let (service, key) = choose(args.local, &prefs, &keys, lm::present(&models, lm::chosen(&models)))?;
    let effort = effort(&prefs);
    let who = match service {
        Service::OpenAi => "gpt-6.2 sol",
        Service::Claude => "opus 5.5",
        Service::Local => lm::chosen(&models).name,
    };
    let width = columns().min(100);
    match args.mode {
        Mode::Ask(question) => {
            let answer = {
                let _w = Waiting::new(&format!("thinking · {who}…"));
                ask(service, key.as_deref(), &format!("{ABOUT}\n\n{PLAIN}"), &question, effort, 200)?
            };
            println!("{}", wrap(&plain(&answer), width));
            Ok(0)
        }
        Mode::Explain(question) => {
            if std::io::stdin().is_terminal() {
                return Err("pipe something in: dmesg | tail -n 30 | ai explain".into());
            }
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input).map_err(|e| format!("reading what was piped in: {e}"))?;
            let count = input.chars().count();
            if count > EXPLAIN_KEEP {
                input = input.chars().skip(count - EXPLAIN_KEEP).collect();
            }
            let ask_what = if question.trim().is_empty() { "What does this mean?".to_owned() } else { question };
            let answer = {
                let _w = Waiting::new(&format!("reading it · {who}…"));
                ask(service, key.as_deref(), &format!("{ABOUT}\n\n{EXPLAIN_RULES} {PLAIN}"), &format!("{ask_what}\n\n```\n{}\n```", input.trim_end()), effort, 200)?
            };
            println!("{}", wrap(&plain(&answer), width));
            Ok(0)
        }
        Mode::Cmd(want) => {
            if want.trim().is_empty() {
                return Err("say what the command should do: ai cmd find the biggest files in /data".into());
            }
            if service == Service::Local {
                return Err(NO_LOCAL_COMMANDS.into());
            }
            let (command, why) = {
                let _w = Waiting::new(&format!("thinking · {who}…"));
                proposal(&ask(service, key.as_deref(), &format!("{ABOUT}\n\n{CMD_RULES}"), &want, effort, 80)?)?
            };
            println!("  $ {command}");
            if !why.is_empty() {
                println!("{}", wrap(&format!("  {why}"), width));
            }
            let go = if dangerous(&command) {
                confirm("this could delete or change things. type yes to run it: ", &["yes"])
            } else {
                confirm("run it? [y/N] ", &["y", "yes"])
            };
            if !go {
                println!("not run");
                return Ok(0);
            }
            let status = Command::new("sh").arg("-c").arg(&command).status().map_err(|e| format!("could not run it: {e}"))?;
            Ok(status.code().unwrap_or(1))
        }
        Mode::App(want) => {
            if want.trim().is_empty() {
                return Err("say what the app should do: ai app a tally of pit batteries and their volts".into());
            }
            if service == Service::Local {
                return Err("the on-device model is too small to write an app you can trust: ai app needs an OpenAI or Claude key (Settings > Keys)".into());
            }
            let (app, script) = {
                let _w = Waiting::new(&format!("writing it · {who}…"));
                app_files(&ask(service, key.as_deref(), &format!("{ABOUT}\n\n{APP_RULES}"), &want, effort, 1600)?)?
            };
            let (id, app) = checked_app(&app)?;
            // All of it shown before anything is written: it runs as root.
            println!("{app}");
            for (n, line) in script.lines().enumerate() {
                println!("{:>3} {line}", n + 1);
            }
            let risky = script.lines().any(dangerous);
            let go = if risky {
                confirm("its script could delete or change things. type yes to put it on Home: ", &["yes"])
            } else {
                confirm("put it on Home? [y/N] ", &["y", "yes"])
            };
            if !go {
                println!("not written");
                return Ok(0);
            }
            let folder = put_app(&apps_dir(), &id, &app, &script)?;
            println!("on Home now, from {} (back to Home to see its tile)", folder.display());
            Ok(0)
        }
        Mode::Help => Ok(0),
    }
}

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(why) => {
            eprintln!("{}", wrap(&format!("ai: {why}"), columns()));
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kv(text: &str) -> Vec<(String, String)> {
        pairs(text)
    }

    #[test]
    fn the_command_line_reads_as_a_question_a_command_or_an_explanation() {
        let s = |a: &[&str]| parse_args(&a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
        assert_eq!(s(&["what", "is", "can"]).mode, Mode::Ask("what is can".into()));
        assert_eq!(s(&["cmd", "find", "big", "files"]).mode, Mode::Cmd("find big files".into()));
        assert_eq!(s(&["explain"]).mode, Mode::Explain(String::new()));
        assert_eq!(s(&["--local", "explain", "why?"]), Args { mode: Mode::Explain("why?".into()), local: Some(true), listen: false });
        assert_eq!(s(&["--listen"]), Args { mode: Mode::Ask(String::new()), local: None, listen: true }, "said, not typed");
        assert_eq!(s(&["-l", "cmd"]).mode, Mode::Cmd(String::new()));
        assert_eq!(s(&[]).mode, Mode::Help);
        assert_eq!(s(&["--help"]).mode, Mode::Help);
        assert_eq!(s(&["help", "me"]).mode, Mode::Help, "help first is help");
    }

    #[test]
    fn the_answerer_is_the_assistants_or_the_model_on_the_handheld() {
        let keys = kv("tba=x\nopenai=sk-o\nclaude=sk-ant-c\n");
        assert_eq!(choose(None, &kv("service=claude"), &keys, false).unwrap(), (Service::Claude, Some("sk-ant-c".into())));
        assert_eq!(choose(None, &[], &keys, false).unwrap().0, Service::OpenAi, "OpenAI first when nothing is chosen");
        assert_eq!(choose(Some(true), &[], &keys, true).unwrap().0, Service::Local, "--local asks the model even with keys");
        assert!(choose(Some(true), &[], &keys, false).unwrap_err().contains("not downloaded"));
        assert_eq!(choose(None, &[], &[], true).unwrap().0, Service::Local, "no key: the model, when it is here");
        assert!(choose(Some(false), &[], &[], true).is_err(), "--cloud with no key");
        assert!(choose(None, &[], &[], false).unwrap_err().contains("no key yet"));
        // 0.4.0's single key counts for the service it belongs to.
        assert_eq!(choose(None, &[], &kv("ai=sk-ant-old"), false).unwrap(), (Service::Claude, Some("sk-ant-old".into())));
        assert_eq!(effort(&kv("effort=light")), "low");
        assert_eq!(effort(&[]), "medium");
    }

    #[test]
    fn each_service_is_asked_in_its_own_shape_and_answers_read() {
        let o = openai_body("sys", "q", "low");
        assert_eq!((o["model"].as_str(), o["store"].as_bool(), o["reasoning"]["effort"].as_str()), (Some("gpt-6.2-sol"), Some(false), Some("low")));
        assert_eq!(o["input"][0]["content"], "q");
        let c = claude_body("sys", "q", "medium");
        assert_eq!((c["model"].as_str(), c["fallbacks"].as_str(), c["output_config"]["effort"].as_str()), (Some("claude-opus-5-5"), Some("default"), Some("medium")));
        assert!(c.get("thinking").is_none() && c.get("temperature").is_none());
        let openai = json!({"output": [{"type": "reasoning"}, {"type": "message", "content": [{"type": "output_text", "text": "Hi."}]}]});
        assert_eq!(answer_of(Service::OpenAi, &openai).unwrap(), "Hi.");
        let claude = json!({"content": [{"type": "thinking", "thinking": ""}, {"type": "text", "text": "Hello."}], "stop_reason": "end_turn"});
        assert_eq!(answer_of(Service::Claude, &claude).unwrap(), "Hello.");
        assert!(answer_of(Service::Claude, &json!({"content": [], "stop_reason": "refusal", "stop_details": {"explanation": "no"}})).unwrap_err().contains("no"));
        let local = json!({"choices": [{"message": {"content": "ok"}}]});
        assert_eq!(answer_of(Service::Local, &local).unwrap(), "ok");
    }

    #[test]
    fn a_proposed_command_is_read_however_it_is_dressed() {
        assert_eq!(proposal("COMMAND: du -a /data | sort -n -r | head -n 10\nWHY: lists the biggest.").unwrap(), ("du -a /data | sort -n -r | head -n 10".into(), "lists the biggest.".into()));
        assert_eq!(proposal("**COMMAND:** `df -h`\n**WHY:** free space").unwrap(), ("df -h".into(), "free space".into()));
        assert_eq!(proposal("Here:\n```sh\nuptime\n```").unwrap().0, "uptime");
        assert_eq!(proposal("COMMAND: none\nWHY: that needs two steps").unwrap_err(), "that needs two steps");
        assert!(proposal("I would rather not.").is_err());
        assert!(NO_LOCAL_COMMANDS.contains("Settings > Keys"));
    }

    #[test]
    fn an_app_written_for_you_is_read_checked_and_put_on_home() {
        let reply = "Here it is.\n\nFILE: app\n```\nid = battery-log\nname = Battery log\nicon = battery_full\nabout = volts of each pit battery\nversion = 1.0.0\nrun = sh ./run.sh\nscreen = own\n```\n\n**FILE: run.sh**\n```sh\n#!/bin/sh\nwhile choice=$(catalyst-ui menu \"Battery log\" \"Add\" \"Show\"); do\n\t:\ndone\n```\n";
        let (app, script) = app_files(reply).unwrap();
        assert!(script.starts_with("#!/bin/sh\n") && script.contains("catalyst-ui menu"));
        let (id, file) = checked_app(&app).unwrap();
        assert_eq!(id, "battery-log");
        assert!(file.contains("screen = own\n") && file.contains("credit = written by the assistant"));
        // What a handheld cannot take.
        assert!(checked_app(&app.replace("battery-log", "../x")).is_err());
        assert!(checked_app(&app.replace("sh ./run.sh", "rm -rf /")).is_err());
        assert!(app_files("FILE: app\n```\nid = x\n```\n").is_err(), "no script, no app");
        // Put on Home once; never over an app already there.
        let dir = std::env::temp_dir().join(format!("ai-app-test-{}", std::process::id()));
        let folder = put_app(&dir, &id, &file, &script).unwrap();
        assert_eq!(std::fs::read_to_string(folder.join("run.sh")).unwrap(), script);
        assert!(put_app(&dir, &id, &file, &script).unwrap_err().contains("already"));
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(parse_args(&["app".into(), "a".into(), "timer".into()]).mode, Mode::App("a timer".into()));
    }

    #[test]
    fn the_listening_meter_is_plain_characters_any_font_has() {
        assert_eq!(meter(0.0), "[............]");
        assert_eq!(meter(0.5), "[||||||......]");
        assert_eq!(meter(2.0), "[||||||||||||]");
        assert!(meter(0.7).is_ascii());
    }

    #[test]
    fn help_fits_the_handheld_terminal() {
        for line in HELP.lines() {
            assert!(line.chars().count() < 53, "wraps at 53 columns: {line:?}");
        }
    }

    #[test]
    fn commands_that_could_change_things_need_a_yes() {
        for c in ["rm -rf /data/logs", "dd if=/dev/zero of=/dev/mmcblk0", "echo x > /etc/hosts", "mkfs.ext4 /dev/sda1", "find /data -name '*.tmp' -delete", "sed -i s/a/b/ f", "git push", "reboot", "kill -9 1", "ls | tee out.txt"] {
            assert!(dangerous(c), "{c}");
        }
        for c in ["du -a /data | sort -n -r | head -n 10", "df -h", "dmesg | tail -n 30 2>&1", "find /data -name '*.wpilog'", "cat /proc/meminfo", "ping -c 4 10.58.5.2"] {
            assert!(!dangerous(c), "{c}");
        }
    }

    #[test]
    fn answers_are_plain_and_fit_the_terminal() {
        assert_eq!(plain("## Fix\n**Do** this:\n- one\n```sh\nls\n```"), "Fix\nDo this:\n· one\nls");
        let wrapped = wrap("the quick brown fox jumps over the lazy dog again and again", 20);
        assert!(wrapped.lines().all(|l| l.chars().count() <= 20), "{wrapped}");
        assert_eq!(wrap("a\n\nb", 53), "a\n\nb", "blank lines kept");
    }

    #[test]
    fn the_key_file_is_for_this_user_only_and_gone_after() {
        let path = private_file("x-api-key: secret\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "x-api-key: secret\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::remove_file(&path).ok();
    }
}
