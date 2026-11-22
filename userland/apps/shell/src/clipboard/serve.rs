//! `catalyst-clip`: the clipboard for shell scripts and the Terminal.
//!
//! ```text
//! catalyst-clip get            the clipboard, as it is
//! catalyst-clip get 3          the third clip in the history (as the panel numbers them)
//! catalyst-clip set [words…]   copy the words, or what comes in (one final line break left off)
//! catalyst-clip set --secret   the same, as a secret: never kept, gone in 30 s
//! catalyst-clip history        the history, a line each: its number (* pinned), its first line
//! catalyst-clip clear          empty the clipboard and the history (pinned clips stay)
//! ```
//!
//! It is the shell's own program (`/usr/bin/catalyst-clip` is a link to
//! `catalyst-shell`), and talks to the running shell over a socket only
//! root can open (`/run/catalyst/clip.sock`), on a thread of the shell's
//! that sleeps until asked. With no shell running, `get` and `history`
//! read the history's file instead. A secret is never handed out: `get`
//! says one is there (exit 3) and nothing more.

use std::io::{Read, Write};
use std::path::PathBuf;

use super::{with, Sensitivity, MOST_BYTES};

/// Where the shell listens.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn socket_path() -> PathBuf {
    if cfg!(target_os = "linux") && std::path::Path::new("/data").is_dir() {
        PathBuf::from("/run/catalyst/clip.sock")
    } else {
        crate::config_dir().join("clip.sock")
    }
}

/// What a request asks, and what is answered: `ok` and what was asked for,
/// or `error` and why, with the exit code a script sees.
#[derive(Debug, PartialEq)]
pub enum Answer {
    Ok(String),
    Error(i32, String),
}

/// Answer one request against the board: its first line says what, and
/// what follows it is the text to set.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn answer(request: &[u8]) -> Answer {
    let text = String::from_utf8_lossy(request);
    let (head, body) = text.split_once('\n').unwrap_or((&text, ""));
    let mut words = head.split_whitespace();
    match (words.next(), words.next()) {
        (Some("get"), None) => with(|b| match (b.plain(), b.secret_left(std::time::Instant::now())) {
            (_, Some(_)) => Answer::Error(3, "the clipboard holds a secret: it is only pasted by hand".into()),
            (Some(text), None) => Answer::Ok(text),
            (None, None) => Answer::Error(1, "the clipboard is empty".into()),
        }),
        (Some("get"), Some(n)) => {
            let Some(at) = n.parse::<usize>().ok().filter(|n| *n > 0) else { return Answer::Error(2, format!("not a clip's number: {n}")) };
            with(|b| b.items().into_iter().nth(at - 1).map_or_else(|| Answer::Error(1, format!("no clip {at}: the history has {}", b.items().len())), |i| Answer::Ok(i.text)))
        }
        (Some("history"), None) => Answer::Ok(with(|b| history_lines(&b.items()))),
        (Some("set"), secret) => {
            let sensitivity = if secret == Some("secret") { Sensitivity::Secret } else { Sensitivity::Normal };
            let text = body.strip_suffix('\n').map(|t| t.strip_suffix('\r').unwrap_or(t)).unwrap_or(body);
            if text.trim().is_empty() {
                return Answer::Error(2, "nothing to copy".into());
            }
            super::copy(text, sensitivity);
            crate::platform::wake();
            Answer::Ok(String::new())
        }
        (Some("clear"), None) => {
            with(|b| b.clear());
            crate::platform::wake();
            Answer::Ok(String::new())
        }
        _ => Answer::Error(2, format!("not understood: {head}")),
    }
}

/// The history a line a clip: its number (a * if pinned) and its first
/// line, with how many lines there are when there are more.
pub fn history_lines(items: &[super::Item]) -> String {
    let mut out = String::new();
    for (i, item) in items.iter().enumerate() {
        let lines = item.text.trim_end().lines().count();
        let first: String = item.text.trim().lines().next().unwrap_or("").chars().filter(|c| !c.is_control()).take(120).collect();
        let more = if lines > 1 { format!(" (+{} lines)", lines - 1) } else { String::new() };
        out.push_str(&format!("{}{}\t{first}{more}\n", i + 1, if item.pinned { "*" } else { "" }));
    }
    out
}

/// Listen for `catalyst-clip`, on a thread that sleeps until asked. On the
/// handheld (and any Unix); a computer's simulator has no socket.
pub fn serve() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::UnixListener;
        let path = socket_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        // One left by a shell before this one.
        std::fs::remove_file(&path).ok();
        let Ok(listener) = UnixListener::bind(&path) else { return };
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).ok();
        std::thread::Builder::new()
            .name("clipboard-socket".into())
            .stack_size(128 * 1024)
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let mut stream = stream;
                    stream.set_read_timeout(Some(std::time::Duration::from_secs(3))).ok();
                    let mut request = Vec::new();
                    // At most a clip's worth and its command line.
                    if (&mut stream).take(MOST_BYTES as u64 + 256).read_to_end(&mut request).is_err() {
                        continue;
                    }
                    let reply = match answer(&request) {
                        Answer::Ok(text) => format!("ok\n{text}"),
                        Answer::Error(code, why) => format!("error {code} {why}\n"),
                    };
                    stream.write_all(reply.as_bytes()).ok();
                }
            })
            .ok();
    }
}

const USAGE: &str = "catalyst-clip: the handheld's clipboard
  get            print the clipboard
  get N          print clip N of the history
  set [words]    copy the words, or what comes in
  set --secret   the same, kept 30 s and never written down
  history        the history, a line each (* pinned)
  clear          empty the clipboard and its history (pinned clips stay)";

/// `catalyst-clip …`: its exit code.
pub fn cli(args: &[String]) -> i32 {
    let mut args = args.iter().map(String::as_str);
    let request = match (args.next(), args.next()) {
        (Some("get"), n) => format!("get {}\n", n.unwrap_or("")),
        (Some("history"), None) => "history\n".to_owned(),
        (Some("clear"), None) => "clear\n".to_owned(),
        (Some("set"), first) => {
            let secret = first == Some("--secret");
            let words: Vec<&str> = first.filter(|_| !secret).into_iter().chain(args).collect();
            let text = if words.is_empty() {
                let mut text = String::new();
                if std::io::stdin().take(MOST_BYTES as u64).read_to_string(&mut text).is_err() {
                    eprintln!("catalyst-clip: what came in is not text");
                    return 2;
                }
                text
            } else {
                super::clamp(&words.join(" "))
            };
            format!("set{}\n{text}", if secret { " secret" } else { "" })
        }
        (Some("-h" | "--help" | "help"), _) => {
            println!("{USAGE}");
            return 0;
        }
        _ => {
            eprintln!("{USAGE}");
            return 2;
        }
    };
    let reply = match ask(&request) {
        Some(reply) => reply,
        // No shell to ask: what it keeps on the card is read instead.
        None if request.starts_with("get") || request.starts_with("history") => {
            match offline(&request, super::Board::open(super::path())) {
                Answer::Ok(text) => format!("ok\n{text}"),
                Answer::Error(code, why) => format!("error {code} {why}\n"),
            }
        }
        None => {
            eprintln!("catalyst-clip: the shell is not running");
            return 1;
        }
    };
    match reply.split_once('\n') {
        Some(("ok", text)) => {
            print!("{text}");
            // A clip printed to a terminal ends its line; piped, it is as it is.
            if !text.is_empty() && !text.ends_with('\n') && is_terminal() {
                println!();
            }
            std::io::stdout().flush().ok();
            0
        }
        Some((head, _)) => {
            let mut parts = head.splitn(3, ' ');
            let (_, code, why) = (parts.next(), parts.next(), parts.next());
            eprintln!("catalyst-clip: {}", why.unwrap_or("the shell did not answer"));
            code.and_then(|c| c.parse().ok()).unwrap_or(1)
        }
        None => {
            eprintln!("catalyst-clip: the shell did not answer");
            1
        }
    }
}

/// A read-only request answered from the history's file.
fn offline(request: &str, mut board: super::Board) -> Answer {
    let head = request.lines().next().unwrap_or("");
    let mut words = head.split_whitespace();
    match (words.next(), words.next()) {
        (Some("get"), None) => board.paste(std::time::Instant::now()).map_or_else(|| Answer::Error(1, "the clipboard is empty".into()), Answer::Ok),
        (Some("get"), Some(n)) => match n.parse::<usize>().ok().filter(|n| *n > 0).and_then(|n| board.items().into_iter().nth(n - 1)) {
            Some(item) => Answer::Ok(item.text),
            None => Answer::Error(1, format!("no clip {n}")),
        },
        _ => Answer::Ok(history_lines(&board.items())),
    }
}

/// Send a request to the running shell; its whole reply.
fn ask(request: &str) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::net::UnixStream;
        let mut stream = UnixStream::connect(socket_path()).ok()?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok();
        stream.write_all(request.as_bytes()).ok()?;
        stream.shutdown(std::net::Shutdown::Write).ok()?;
        let mut reply = String::new();
        stream.take(MOST_BYTES as u64 + 256).read_to_string(&mut reply).ok()?;
        Some(reply)
    }
    #[cfg(not(unix))]
    {
        let _ = request;
        None
    }
}

fn is_terminal() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_sets_gets_and_lists_but_never_reads_a_secret() {
        assert_eq!(answer(b"get\n"), Answer::Error(1, "the clipboard is empty".into()));
        assert_eq!(answer(b"set\nswap battery\n"), Answer::Ok(String::new()));
        assert_eq!(answer(b"get\n"), Answer::Ok("swap battery".into()), "one final line break left off");
        assert_eq!(answer(b"set\nline one\nline two"), Answer::Ok(String::new()));
        assert_eq!(answer(b"history\n"), Answer::Ok("1\tline one (+1 lines)\n2\tswap battery\n".into()));
        assert_eq!(answer(b"get 2\n"), Answer::Ok("swap battery".into()));
        assert!(matches!(answer(b"get 9\n"), Answer::Error(1, _)));
        assert!(matches!(answer(b"get x\n"), Answer::Error(2, _)));
        assert_eq!(answer(b"set secret\nhunter2\n"), Answer::Ok(String::new()));
        assert!(matches!(answer(b"get\n"), Answer::Error(3, _)), "a secret is not handed to a script");
        assert!(!answer_text(b"history\n").contains("hunter2"));
        assert_eq!(crate::clipboard::paste_text().as_deref(), Some("hunter2"), "it is pasted by hand");
        assert!(matches!(answer(b"set\n  \n"), Answer::Error(2, _)));
        assert_eq!(answer(b"clear\n"), Answer::Ok(String::new()));
        assert_eq!(answer(b"history\n"), Answer::Ok(String::new()));
        assert!(matches!(answer(b"frobnicate\n"), Answer::Error(2, _)));
    }

    /// The socket itself, as `catalyst-clip` uses it (on Linux: the ARM
    /// tests under emulation run it).
    #[cfg(unix)]
    #[test]
    fn catalyst_clip_talks_to_the_shell_over_its_socket() {
        crate::shell::tests::sandbox();
        serve();
        let set = ask("set\n10.58.5.2\n").expect("the shell answers");
        assert_eq!(set, "ok\n");
        assert_eq!(ask("get\n").as_deref(), Some("ok\n10.58.5.2"));
        assert_eq!(cli(&["set".into(), "--secret".into(), "hunter2".into()]), 0);
        assert_eq!(cli(&["get".into()]), 3, "a secret is never handed out");
        assert_eq!(cli(&["frobnicate".into()]), 2);
        let mode = std::fs::metadata(socket_path()).map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o777).unwrap();
        assert_eq!(mode, 0o600, "root's alone");
    }

    fn answer_text(request: &[u8]) -> String {
        match answer(request) {
            Answer::Ok(t) | Answer::Error(_, t) => t,
        }
    }

    #[test]
    fn the_history_lists_one_line_a_clip_pinned_marked() {
        let items = vec![
            crate::clipboard::Item { text: "10.58.5.2".into(), pinned: true, at: 0 },
            crate::clipboard::Item { text: "a\tb\nc\n".into(), pinned: false, at: 0 },
        ];
        assert_eq!(history_lines(&items), "1*\t10.58.5.2\n2\tab (+1 lines)\n");
    }
}
