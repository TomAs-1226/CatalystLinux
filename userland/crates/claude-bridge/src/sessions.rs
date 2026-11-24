//! Claude Code's sessions on this computer, from its transcripts:
//! `%USERPROFILE%\.claude\projects\<project>\<session id>.jsonl`
//! (`CLAUDE_CONFIG_DIR` moves `.claude`).
//!
//! The transcript format is Claude Code's own and changes between versions
//! (its documentation says so), so this reads it defensively and as little
//! as it can: a session's start (its folder, `cwd`, and its first prompt)
//! and its end (the newest title, set with /rename or made by Claude Code,
//! and a folder it moved to). Transcripts reach hundreds of megabytes, so
//! never more than [`HEAD`] from the start and [`TAIL`] from the end are
//! read, and what was read is kept until the file changes.
//!
//! [`history`] turns a transcript's end into the rows the handheld shows,
//! the same rows a turn streams ([`crate::stream`]).

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::{json, Value};

use crate::summary;

/// How much of a transcript's start is read for its folder and first prompt.
const HEAD: u64 = 256 * 1024;
/// How much of its end is read for its title.
const TAIL: u64 = 256 * 1024;
/// How much of its end the history is made from.
const HISTORY_TAIL: u64 = 3 * 1024 * 1024;
/// The longest title or first prompt shown.
const TITLE_MOST: usize = 90;

/// One session, as the list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub id: String,
    pub title: String,
    /// The folder it works in (its transcript's `cwd`).
    pub cwd: String,
    /// That folder's name.
    pub project: String,
    /// When its transcript last changed, in seconds since 1970.
    pub last: i64,
    pub path: PathBuf,
}

impl Session {
    pub fn json(&self) -> Value {
        json!({"id": self.id, "title": self.title, "project": self.project, "cwd": self.cwd, "last": self.last})
    }
}

/// Where the transcripts are.
pub fn projects_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return PathBuf::from(dir).join("projects");
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default();
    home.join(".claude").join("projects")
}

/// What was read of a transcript, kept until it changes.
#[derive(Default)]
pub struct Cache {
    seen: HashMap<PathBuf, ((u64, i64), Option<Session>)>,
}

fn read_range(path: &Path, from: u64, most: u64) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    f.seek(SeekFrom::Start(from)).ok()?;
    let mut buf = Vec::new();
    f.take(most).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Whole lines of a part of a file: one cut at its start (when it does
/// not begin the file) or at its end (when the file goes on) left out.
fn whole_lines(text: &str, cut_start: bool, cut_end: bool) -> Vec<&str> {
    let mut lines: Vec<&str> = text.split('\n').collect();
    if cut_end && !lines.is_empty() {
        lines.pop();
    }
    if cut_start && !lines.is_empty() {
        lines.remove(0);
    }
    lines.into_iter().filter(|l| !l.trim().is_empty()).collect()
}

/// A user message's own words: none for Claude Code's own entries (meta,
/// a subagent's, a command's output), tool results, or reminders.
pub fn prompt_text(v: &Value) -> Option<String> {
    if v["type"] != "user" || v["isMeta"] == true || v["isSidechain"] == true || v["isCompactSummary"] == true || v["isVisibleInTranscriptOnly"] == true {
        return None;
    }
    let content = &v["message"]["content"];
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => {
            if blocks.iter().any(|b| b["type"] == "tool_result") {
                return None;
            }
            blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n")
        }
        _ => return None,
    };
    let text = text.trim();
    let machine = ["<command-", "<local-command", "<system-reminder>", "<bash-", "<user-memory", "Caveat:", "[Request interrupted"];
    if text.is_empty() || machine.iter().any(|m| text.starts_with(m)) {
        return None;
    }
    Some(text.to_owned())
}

/// A session from its transcript, if it holds a conversation.
fn read_session(path: &Path, len: u64, last: i64) -> Option<Session> {
    let id = path.file_stem()?.to_str()?.to_owned();
    let head = read_range(path, 0, HEAD)?;
    let head_lines = whole_lines(&head, false, len > HEAD);
    let (mut cwd, mut first) = (String::new(), None);
    for line in &head_lines {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if cwd.is_empty() {
            if let Some(c) = v["cwd"].as_str() {
                cwd = c.to_owned();
            }
        }
        if first.is_none() {
            first = prompt_text(&v);
        }
        if !cwd.is_empty() && first.is_some() {
            break;
        }
    }
    // The end: the newest title of each kind, and where it moved to.
    let from = len.saturating_sub(TAIL);
    let tail = if from == 0 { head.clone() } else { read_range(path, from, TAIL)? };
    let (mut custom, mut ai) = (None, None);
    for line in whole_lines(&tail, from > 0, false) {
        if !(line.contains("custom-title") || line.contains("ai-title") || line.contains("relocated")) {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        match v["type"].as_str() {
            Some("custom-title") => custom = v["customTitle"].as_str().map(str::to_owned).or(custom),
            Some("ai-title") => ai = v["aiTitle"].as_str().map(str::to_owned).or(ai),
            Some("relocated") => {
                if let Some(c) = v["relocatedCwd"].as_str() {
                    cwd = c.to_owned();
                }
            }
            _ => {}
        }
    }
    let title = custom.or(ai).or(first)?;
    let title = summary::cut(&summary::one_line(&title), TITLE_MOST);
    let project = summary::file_name(&cwd);
    Some(Session { id, title, cwd, project, last, path: path.to_owned() })
}

/// Whether a name is a session id as Claude Code makes them (a UUID), so
/// nothing else in a project folder is taken for one, and no id from the
/// handheld can name a path.
pub fn is_session_id(id: &str) -> bool {
    id.len() == 36 && id.chars().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

/// Every session under `dir`, newest first.
pub fn list(dir: &Path, cache: &mut Cache) -> Vec<Session> {
    let mut found = Vec::new();
    for project in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if !project.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        for entry in std::fs::read_dir(project.path()).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") || !path.file_stem().and_then(|s| s.to_str()).is_some_and(is_session_id) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let last = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
            let stamp = (meta.len(), last);
            let session = match cache.seen.get(&path) {
                Some((s, session)) if *s == stamp => session.clone(),
                _ => {
                    let session = read_session(&path, meta.len(), last);
                    cache.seen.insert(path.clone(), (stamp, session.clone()));
                    session
                }
            };
            found.extend(session);
        }
    }
    found.sort_by(|a, b| b.last.cmp(&a.last).then_with(|| a.id.cmp(&b.id)));
    found
}

/// The session with this id, if there is one.
pub fn find(dir: &Path, cache: &mut Cache, id: &str) -> Option<Session> {
    if !is_session_id(id) {
        return None;
    }
    list(dir, cache).into_iter().find(|s| s.id == id)
}

/// The folders sessions have worked in, newest first, each once.
pub fn projects(sessions: &[Session]) -> Vec<(String, String, i64)> {
    let mut out: Vec<(String, String, i64)> = Vec::new();
    for s in sessions {
        if s.cwd.is_empty() || out.iter().any(|(c, ..)| c.eq_ignore_ascii_case(&s.cwd)) {
            continue;
        }
        out.push((s.cwd.clone(), s.project.clone(), s.last));
    }
    out
}

// -------------------------------------------------------------- history ---

/// The rows of a conversation, as the handheld shows them.
#[derive(Default)]
pub struct Rows {
    pub items: Vec<Value>,
    index: HashMap<String, usize>,
}

impl Rows {
    /// A row added, or put in place of the one with its id.
    pub fn put(&mut self, item: Value) {
        let id = item["id"].as_str().unwrap_or_default().to_owned();
        match self.index.get(&id) {
            Some(&i) => self.items[i] = item,
            None => {
                self.index.insert(id, self.items.len());
                self.items.push(item);
            }
        }
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Value> {
        self.index.get(id).copied().map(|i| &mut self.items[i])
    }
}

/// A row: who, its words, and for a tool its symbol, state and detail.
pub fn row(id: &str, kind: &str, text: &str) -> Value {
    json!({"id": id, "k": kind, "text": text})
}

pub fn tool_row(id: &str, shown: &summary::Shown, state: &str) -> Value {
    json!({"id": id, "k": "tool", "text": shown.line, "icon": shown.icon, "state": state, "detail": shown.detail})
}

/// A tool's result taken into its row: done or failed, and what it said.
pub fn finish_tool(row: &mut Value, is_error: bool, result: &str, denied: bool) {
    row["state"] = Value::from(if denied {
        "denied"
    } else if is_error {
        "error"
    } else {
        "ok"
    });
    if !result.is_empty() {
        let detail = row["detail"].as_str().unwrap_or_default().to_owned();
        let joined = if detail.is_empty() { result.to_owned() } else { format!("{detail}\n\n{result}") };
        row["detail"] = Value::from(summary::cut(&joined, summary::DETAIL_MOST));
    }
}

/// One transcript entry (or one stream-json message: they share these
/// shapes) taken into the rows.
pub fn take_entry(rows: &mut Rows, v: &Value, cwd: &str) {
    if v["isSidechain"] == true || v["parent_tool_use_id"].is_string() {
        return;
    }
    match v["type"].as_str() {
        Some("user") => {
            if let Some(text) = prompt_text(v) {
                let id = v["uuid"].as_str().map_or_else(|| format!("you-{}", rows.items.len()), str::to_owned);
                rows.put(row(&id, "you", &summary::cut(&text, summary::DETAIL_MOST)));
                return;
            }
            for b in v["message"]["content"].as_array().into_iter().flatten() {
                if b["type"] != "tool_result" {
                    continue;
                }
                let id = b["tool_use_id"].as_str().unwrap_or_default();
                let result = summary::result_text(&b["content"]);
                let denied = b["is_error"] == true && (result.contains("permission") || result.contains("denied") || result.contains("not allowed"));
                if let Some(r) = rows.get_mut(id) {
                    finish_tool(r, b["is_error"] == true, &result, denied);
                }
            }
        }
        Some("assistant") => {
            let base = v["message"]["id"].as_str().or(v["uuid"].as_str()).unwrap_or("a").to_owned();
            for (i, b) in v["message"]["content"].as_array().into_iter().flatten().enumerate() {
                match b["type"].as_str() {
                    Some("text") => {
                        let text = b["text"].as_str().unwrap_or_default().trim();
                        if !text.is_empty() {
                            let id = format!("{base}:{}", v["uuid"].as_str().map_or(i.to_string(), |u| format!("{u}:{i}")));
                            rows.put(row(&id, "claude", &summary::cut(text, 20_000)));
                        }
                    }
                    Some("tool_use") => {
                        let id = b["id"].as_str().unwrap_or_default();
                        let shown = summary::shown(b["name"].as_str().unwrap_or_default(), &b["input"], cwd);
                        // A row already finished (a transcript read twice) keeps its state.
                        if rows.get_mut(id).is_none() {
                            rows.put(tool_row(id, &shown, "run"));
                        }
                    }
                    _ => {}
                }
            }
        }
        Some("system") if v["subtype"] == "compact_boundary" => {
            let id = v["uuid"].as_str().unwrap_or("compact").to_owned();
            rows.put(row(&id, "note", "earlier conversation summarized"));
        }
        _ => {}
    }
}

/// The newest `most` rows of a session's transcript.
pub fn history(session: &Session, most: usize) -> Vec<Value> {
    let len = std::fs::metadata(&session.path).map_or(0, |m| m.len());
    let from = len.saturating_sub(HISTORY_TAIL);
    let text = read_range(&session.path, from, HISTORY_TAIL).unwrap_or_default();
    let mut rows = Rows::default();
    for line in whole_lines(&text, from > 0, false) {
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            take_entry(&mut rows, &v, &session.cwd);
        }
    }
    // A tool still "running" in a transcript that is not being written to
    // was cut short when its session stopped.
    for r in &mut rows.items {
        if r["state"] == "run" {
            r["state"] = Value::from("cut");
        }
    }
    let skip = rows.items.len().saturating_sub(most);
    rows.items.split_off(skip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_uuids_are_sessions() {
        assert!(is_session_id("3f0c2a9e-1b2c-4d5e-8f90-a1b2c3d4e5f6"));
        assert!(!is_session_id("../../etc/passwd-0000-0000-000000000000"));
        assert!(!is_session_id("3f0c2a9e-1b2c-4d5e-8f90-a1b2c3d4e5f"));
        assert!(!is_session_id("3f0c2a9e_1b2c-4d5e-8f90-a1b2c3d4e5f6"));
    }

    #[test]
    fn machine_words_are_not_prompts() {
        let user = |content: Value| json!({"type": "user", "message": {"role": "user", "content": content}});
        assert_eq!(prompt_text(&user(json!("fix the camera"))).as_deref(), Some("fix the camera"));
        assert_eq!(prompt_text(&user(json!([{"type": "text", "text": "hello"}]))).as_deref(), Some("hello"));
        assert_eq!(prompt_text(&user(json!("<command-name>/clear</command-name>"))), None);
        assert_eq!(prompt_text(&user(json!([{"type": "tool_result", "tool_use_id": "t", "content": "ok"}]))), None);
        let mut meta = user(json!("x"));
        meta["isMeta"] = json!(true);
        assert_eq!(prompt_text(&meta), None);
    }
}
