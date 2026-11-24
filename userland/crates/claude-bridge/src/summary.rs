//! What the handheld is shown of a tool call: one line for the transcript
//! ("edited camera.rs +12 −3", "ran cargo test"), the detail Enter opens,
//! and, for a permission request, what it would do and where.
//!
//! Only text goes to the handheld, and only as much as its screen can use:
//! a line of [`LINE_MOST`] characters, a detail of [`DETAIL_MOST`].

use serde_json::Value;

/// The longest one-line summary.
pub const LINE_MOST: usize = 96;
/// The longest detail.
pub const DETAIL_MOST: usize = 4000;
/// Lines of a diff or an output kept in a detail.
const DETAIL_LINES: usize = 40;

/// A tool call as a transcript row: its symbol, its line and its detail.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub icon: &'static str,
    pub line: String,
    pub detail: String,
}

/// A permission request as the handheld's sheet shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Asked {
    /// What it would do: "run a command", "edit a file".
    pub what: String,
    /// The command, the file or the address, whole (the sheet wraps it).
    pub line: String,
    /// Where: the project folder, or that it is outside it.
    pub place: String,
    /// More, for Enter on the sheet: the edit, the whole input.
    pub detail: String,
}

/// At most `most` characters, an ellipsis at the end if cut.
pub fn cut(s: &str, most: usize) -> String {
    if s.chars().count() <= most {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(most.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// One line: whitespace runs made single spaces.
pub fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The last part of a path, either slash.
pub fn file_name(path: &str) -> String {
    path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(path).to_owned()
}

/// A shell command as the line says it: its first line, without a leading
/// `cd somewhere &&`.
fn command_words(command: &str) -> String {
    let first = command.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let mut c = first;
    for joiner in ["&&", ";"] {
        if let Some((head, rest)) = c.split_once(joiner) {
            let head = head.trim();
            if head.starts_with("cd ") || head.starts_with("Set-Location ") || head.starts_with("pushd ") {
                c = rest.trim();
            }
        }
    }
    let c = c.trim_start_matches("& ").trim();
    let more = if command.trim().lines().count() > 1 { " …" } else { "" };
    format!("{}{more}", one_line(c))
}

/// Lines added and removed between two texts, and the changed lines in
/// order (`− ` and `+ `): a line diff (longest common subsequence) of what
/// is left once the lines they begin and end with alike are set aside, or,
/// past [`DIFF_MOST`] lines, those left lines themselves.
pub fn added_removed(old: &str, new: &str) -> (usize, usize, Vec<String>) {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let tail = a[head..].iter().rev().zip(b[head..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (a, b) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    let mut diff = Vec::new();
    if a.len() > DIFF_MOST || b.len() > DIFF_MOST {
        diff.extend(a.iter().map(|l| format!("− {l}")));
        diff.extend(b.iter().map(|l| format!("+ {l}")));
        return (b.len(), a.len(), diff);
    }
    // lcs[i][j]: the longest common run of a[i..] and b[j..].
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![0u16; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i * (m + 1) + j] = if a[i] == b[j] { lcs[(i + 1) * (m + 1) + j + 1] + 1 } else { lcs[(i + 1) * (m + 1) + j].max(lcs[i * (m + 1) + j + 1]) };
        }
    }
    let (mut i, mut j, mut added, mut removed) = (0, 0, 0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            i += 1;
            j += 1;
        } else if i < n && (j == m || lcs[(i + 1) * (m + 1) + j] >= lcs[i * (m + 1) + j + 1]) {
            // Removed lines before the lines that take their place.
            diff.push(format!("− {}", a[i]));
            removed += 1;
            i += 1;
        } else {
            diff.push(format!("+ {}", b[j]));
            added += 1;
            j += 1;
        }
    }
    (added, removed, diff)
}

/// The most lines a side of an edit is diffed line by line.
const DIFF_MOST: usize = 400;

fn str_of<'a>(input: &'a Value, key: &str) -> &'a str {
    input[key].as_str().unwrap_or("")
}

fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?', '#']).next().unwrap_or(rest).to_owned()
}

/// A path as the project sees it: relative inside `cwd`, whole outside.
pub fn relative(path: &str, cwd: &str) -> (String, bool) {
    let norm = |p: &str| p.replace('\\', "/").trim_end_matches('/').to_lowercase();
    let (p, c) = (norm(path), norm(cwd));
    if !c.is_empty() && (p == c || p.starts_with(&format!("{c}/"))) {
        let shown = path.replace('\\', "/");
        let rel = shown.get(c.len()..).unwrap_or("").trim_start_matches('/');
        return (if rel.is_empty() { ".".into() } else { rel.to_owned() }, true);
    }
    // A path with no root is the project's.
    let rooted = p.starts_with('/') || p.get(1..2) == Some(":") || p.starts_with('~');
    (path.to_owned(), !rooted)
}

fn trim_detail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let kept = if lines.len() > DETAIL_LINES { format!("…\n{}", lines[lines.len() - DETAIL_LINES..].join("\n")) } else { lines.join("\n") };
    // The end is what matters in an output: keep it when cutting.
    if kept.chars().count() > DETAIL_MOST {
        let skip = kept.chars().count() - DETAIL_MOST + 1;
        return format!("…{}", kept.chars().skip(skip).collect::<String>());
    }
    kept
}

fn first_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = lines.iter().take(n).copied().collect::<Vec<_>>().join("\n");
    if lines.len() > n {
        out.push_str(&format!("\n… {} more lines", lines.len() - n));
    }
    cut(&out, DETAIL_MOST)
}

/// The MCP server and tool of an `mcp__server__tool` name.
fn mcp_parts(name: &str) -> Option<(&str, &str)> {
    name.strip_prefix("mcp__")?.split_once("__")
}

/// A tool call as its transcript row.
pub fn shown(name: &str, input: &Value, cwd: &str) -> Shown {
    let path = str_of(input, "file_path");
    let (rel, _) = relative(path, cwd);
    let (icon, line, detail) = match name {
        "Bash" | "PowerShell" => {
            let command = str_of(input, "command");
            ("terminal", format!("ran {}", command_words(command)), format!("$ {}", cut(command, DETAIL_MOST)))
        }
        "Edit" => {
            let (add, rm, diff) = added_removed(str_of(input, "old_string"), str_of(input, "new_string"));
            ("edit", format!("edited {} +{add} −{rm}", file_name(path)), format!("{rel}\n{}", first_lines(&diff.join("\n"), DETAIL_LINES)))
        }
        "MultiEdit" => {
            let (mut add, mut rm, mut diff) = (0, 0, Vec::new());
            for e in input["edits"].as_array().into_iter().flatten() {
                let (a, r, d) = added_removed(str_of(e, "old_string"), str_of(e, "new_string"));
                add += a;
                rm += r;
                diff.extend(d);
            }
            ("edit", format!("edited {} +{add} −{rm}", file_name(path)), format!("{rel}\n{}", first_lines(&diff.join("\n"), DETAIL_LINES)))
        }
        "Write" => {
            let content = str_of(input, "content");
            ("edit_note", format!("wrote {} +{}", file_name(path), content.lines().count()), format!("{rel}\n{}", first_lines(content, 20)))
        }
        "NotebookEdit" => {
            let nb = str_of(input, "notebook_path");
            ("edit", format!("edited {}", file_name(nb)), relative(nb, cwd).0)
        }
        "Read" => {
            let range = match (input["offset"].as_u64(), input["limit"].as_u64()) {
                (Some(o), Some(l)) => format!(" lines {o}–{}", o + l),
                (Some(o), None) => format!(" from line {o}"),
                _ => String::new(),
            };
            ("description", format!("read {}{range}", file_name(path)), rel)
        }
        "Grep" => {
            let pattern = str_of(input, "pattern");
            let place = str_of(input, "path");
            let in_place = if place.is_empty() { String::new() } else { format!(" in {}", file_name(place)) };
            ("search", format!("searched for “{}”{in_place}", cut(&one_line(pattern), 40)), format!("{pattern}\n{}", relative(place, cwd).0))
        }
        "Glob" => ("search", format!("looked for {}", cut(str_of(input, "pattern"), 48)), str_of(input, "pattern").to_owned()),
        "WebFetch" => ("language", format!("fetched {}", host_of(str_of(input, "url"))), str_of(input, "url").to_owned()),
        "WebSearch" => ("travel_explore", format!("searched the web for “{}”", cut(&one_line(str_of(input, "query")), 40)), str_of(input, "query").to_owned()),
        "TodoWrite" => {
            let todos: Vec<String> = input["todos"].as_array().into_iter().flatten().map(|t| format!("{} {}", if t["status"] == "completed" { "✓" } else { "·" }, str_of(t, "content"))).collect();
            ("checklist", "updated the to-do list".to_owned(), first_lines(&todos.join("\n"), 20))
        }
        "Task" | "Agent" => ("account_tree", format!("asked a subagent: {}", cut(&one_line(str_of(input, "description")), 48)), cut(str_of(input, "prompt"), 1200)),
        "Skill" => ("auto_awesome", format!("used the {} skill", str_of(input, "skill")), String::new()),
        _ => match mcp_parts(name) {
            Some((server, tool)) => ("api", format!("used {server}'s {tool}"), cut(&input.to_string(), 1200)),
            None => ("build", format!("used {}", name.to_lowercase()), cut(&input.to_string(), 1200)),
        },
    };
    Shown { icon, line: cut(&line, LINE_MOST), detail }
}

/// What a tool's result adds to its detail: its words, the end kept.
pub fn result_text(content: &Value) -> String {
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    };
    trim_detail(text.trim_end())
}

/// A permission request as the sheet asks it.
pub fn asked(name: &str, input: &Value, cwd: &str) -> Asked {
    let project = file_name(cwd);
    let place_of = |path: &str| -> (String, String) {
        let (rel, inside) = relative(path, cwd);
        if inside {
            (rel, format!("in {project}"))
        } else {
            (path.to_owned(), format!("outside {project}"))
        }
    };
    let s = shown(name, input, cwd);
    let (what, line, place) = match name {
        "Bash" | "PowerShell" => ("run a command".to_owned(), str_of(input, "command").trim().to_owned(), format!("in {project}")),
        "Edit" | "MultiEdit" | "NotebookEdit" => {
            let p = if name == "NotebookEdit" { str_of(input, "notebook_path") } else { str_of(input, "file_path") };
            let (line, place) = place_of(p);
            ("edit a file".to_owned(), line, place)
        }
        "Write" => {
            let (line, place) = place_of(str_of(input, "file_path"));
            ("write a file".to_owned(), line, place)
        }
        "Read" => {
            let (line, place) = place_of(str_of(input, "file_path"));
            ("read a file".to_owned(), line, place)
        }
        "Grep" | "Glob" => {
            let p = str_of(input, "path");
            let (_, place) = if p.is_empty() { (String::new(), format!("in {project}")) } else { place_of(p) };
            ("search files".to_owned(), str_of(input, "pattern").to_owned(), place)
        }
        "WebFetch" => ("fetch a page".to_owned(), str_of(input, "url").to_owned(), "on the web".to_owned()),
        "WebSearch" => ("search the web".to_owned(), str_of(input, "query").to_owned(), "on the web".to_owned()),
        _ => match mcp_parts(name) {
            Some((server, tool)) => (format!("use {server}'s {tool}"), cut(&input.to_string(), 600), format!("for {project}")),
            None => (format!("use {}", name.to_lowercase()), cut(&input.to_string(), 600), format!("for {project}")),
        },
    };
    Asked { what, line: cut(&line, 2000), place, detail: s.detail }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CWD: &str = "C:\\Users\\User\\CatalystLinux";

    #[test]
    fn rows_say_what_was_done_in_a_few_words() {
        let edit = json!({"file_path": "C:\\Users\\User\\CatalystLinux\\userland\\apps\\shell\\src\\camera.rs", "old_string": "a\nb\nc\nd", "new_string": "a\nB\nB2\nB3\nd"});
        let s = shown("Edit", &edit, CWD);
        assert_eq!((s.icon, s.line.as_str()), ("edit", "edited camera.rs +3 −2"));
        assert!(s.detail.starts_with("userland/apps/shell/src/camera.rs\n− b\n− c\n+ B"), "{}", s.detail);
        assert_eq!(shown("Bash", &json!({"command": "cd userland && cargo test -p catalyst-shell -q"}), CWD).line, "ran cargo test -p catalyst-shell -q");
        assert_eq!(shown("Bash", &json!({"command": "cargo test\ncargo build"}), CWD).line, "ran cargo test …");
        assert_eq!(shown("PowerShell", &json!({"command": "& \"$env:USERPROFILE\\.cargo\\bin\\cargo.exe\" test"}), CWD).line, "ran \"$env:USERPROFILE\\.cargo\\bin\\cargo.exe\" test");
        assert_eq!(shown("Write", &json!({"file_path": "C:\\x\\notes.md", "content": "1\n2\n3"}), CWD).line, "wrote notes.md +3");
        assert_eq!(shown("Read", &json!({"file_path": "/a/b/main.rs", "offset": 10, "limit": 40}), CWD).line, "read main.rs lines 10–50");
        assert_eq!(shown("Grep", &json!({"pattern": "fn  draw", "path": "src"}), CWD).line, "searched for “fn draw” in src");
        assert_eq!(shown("WebFetch", &json!({"url": "https://docs.rs/x/y?z"}), CWD).line, "fetched docs.rs");
        assert_eq!(shown("mcp__github__create_issue", &json!({}), CWD).line, "used github's create_issue");
        assert_eq!(shown("Frobnicate", &json!({}), CWD).line, "used frobnicate");
        let long = "x".repeat(300);
        assert_eq!(shown("Bash", &json!({"command": long}), CWD).line.chars().count(), LINE_MOST);
    }

    #[test]
    fn a_request_says_what_and_where() {
        let a = asked("Bash", &json!({"command": "cargo test"}), CWD);
        assert_eq!((a.what.as_str(), a.line.as_str(), a.place.as_str()), ("run a command", "cargo test", "in CatalystLinux"));
        let a = asked("Edit", &json!({"file_path": "C:\\Users\\User\\.bashrc", "old_string": "", "new_string": "x"}), CWD);
        assert_eq!((a.what.as_str(), a.line.as_str(), a.place.as_str()), ("edit a file", "C:\\Users\\User\\.bashrc", "outside CatalystLinux"));
        let a = asked("Write", &json!({"file_path": "c:/users/user/catalystlinux/README.md", "content": ""}), CWD);
        assert_eq!((a.line.as_str(), a.place.as_str()), ("README.md", "in CatalystLinux"));
        let a = asked("WebFetch", &json!({"url": "https://example.com"}), CWD);
        assert_eq!(a.place, "on the web");
    }

    #[test]
    fn outputs_keep_their_end() {
        let out: String = (1..=100).map(|i| format!("line {i}\n")).collect();
        let t = result_text(&json!([{"type": "text", "text": out}]));
        assert!(t.starts_with("…\nline 61") && t.ends_with("line 100"), "{t}");
        assert_eq!(result_text(&json!("ok")), "ok");
    }
}
