//! A turn's output from `claude -p --output-format stream-json --verbose
//! --include-partial-messages`, one JSON object a line, turned into what
//! the handheld is sent: rows (new, or in place of one with the same id),
//! words appended to the row being written, and how the turn stands.
//!
//! The messages and fields read, as the Agent SDK's reference documents
//! them (`SDKMessage`): `system` `init` (`session_id`), `api_retry`
//! (`attempt`) and `compact_boundary`; `stream_event` carrying the API's
//! `message_start`, `content_block_start` and `content_block_delta`
//! (`text_delta`); `assistant` (`message.id`, `message.content`, `uuid`,
//! `error`); `user` with `tool_result` blocks; and `result` (`subtype`,
//! `is_error`, `errors`). Anything else is let go, so a newer CLI's new
//! messages change nothing.
//!
//! Text streams into a row of its own while it is written; when the whole
//! message comes, its row takes the message's own id (the one its
//! transcript keeps, so the history read later is the same rows) in place
//! of the streaming one (`replaces`).

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

use crate::sessions::{self, Rows};
use crate::summary;

/// What a line of the stream gives the handheld.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    /// Claude Code has begun: its session's id.
    Session(String),
    /// A row, new or in place of the one with its id (or of `replaces`).
    Row(Value),
    /// Words written on to the row with this id.
    Append { id: String, text: String },
    /// How it is going, while it works: "retrying · attempt 2".
    Status(String),
    /// The turn is over: whether it went well, and in a few words if not.
    Done { ok: bool, words: String },
}

/// One turn's stream, as it is read.
#[derive(Default)]
pub struct Turn {
    cwd: String,
    rows: Rows,
    /// The message being streamed, and its text blocks' rows by index.
    streaming: Option<String>,
    blocks: HashMap<u64, String>,
    /// Each message's streamed text rows, in order, and how many of them
    /// its whole message has taken over.
    streamed: HashMap<String, Vec<String>>,
    taken: HashMap<String, usize>,
    /// Tool calls the handheld (or nobody, in time) did not allow.
    pub denied: HashSet<String>,
    pub done: bool,
}

impl Turn {
    pub fn new(cwd: &str) -> Turn {
        Turn { cwd: cwd.to_owned(), ..Turn::default() }
    }

    /// One line of the stream.
    pub fn line(&mut self, line: &str) -> Vec<Out> {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { return Vec::new() };
        // A subagent's own messages stay inside its row.
        if v["parent_tool_use_id"].is_string() {
            return Vec::new();
        }
        match v["type"].as_str() {
            Some("system") => self.system(&v),
            Some("stream_event") => self.partial(&v["event"]),
            Some("assistant") => self.assistant(&v),
            Some("user") => self.user(&v),
            Some("result") => self.result(&v),
            _ => Vec::new(),
        }
    }

    fn system(&mut self, v: &Value) -> Vec<Out> {
        match v["subtype"].as_str() {
            Some("init") => v["session_id"].as_str().map(|s| vec![Out::Session(s.to_owned())]).unwrap_or_default(),
            Some("api_retry") => vec![Out::Status(format!("retrying · attempt {}", v["attempt"].as_u64().unwrap_or(1)))],
            Some("compact_boundary") => vec![Out::Row(sessions::row(v["uuid"].as_str().unwrap_or("compact"), "note", "earlier conversation summarized"))],
            _ => Vec::new(),
        }
    }

    fn partial(&mut self, e: &Value) -> Vec<Out> {
        match e["type"].as_str() {
            Some("message_start") => {
                self.streaming = e["message"]["id"].as_str().map(str::to_owned);
                self.blocks.clear();
                Vec::new()
            }
            Some("content_block_start") if e["content_block"]["type"] == "text" => {
                let (Some(msg), Some(index)) = (self.streaming.clone(), e["index"].as_u64()) else { return Vec::new() };
                let id = format!("{msg}:s{index}");
                self.blocks.insert(index, id.clone());
                self.streamed.entry(msg).or_default().push(id.clone());
                let text = e["content_block"]["text"].as_str().unwrap_or_default();
                vec![Out::Row(sessions::row(&id, "claude", text))]
            }
            Some("content_block_delta") if e["delta"]["type"] == "text_delta" => {
                let Some(id) = e["index"].as_u64().and_then(|i| self.blocks.get(&i)) else { return Vec::new() };
                let text = e["delta"]["text"].as_str().unwrap_or_default();
                if text.is_empty() {
                    return Vec::new();
                }
                vec![Out::Append { id: id.clone(), text: text.to_owned() }]
            }
            _ => Vec::new(),
        }
    }

    fn assistant(&mut self, v: &Value) -> Vec<Out> {
        let mut out = Vec::new();
        if let Some(error) = v["error"].as_str() {
            out.push(Out::Row(sessions::row(&format!("error-{}", v["uuid"].as_str().unwrap_or("x")), "note", &error_words(error))));
        }
        let msg = v["message"]["id"].as_str().unwrap_or("a").to_owned();
        let uuid = v["uuid"].as_str().unwrap_or("u");
        for (i, b) in v["message"]["content"].as_array().into_iter().flatten().enumerate() {
            match b["type"].as_str() {
                Some("text") => {
                    let text = b["text"].as_str().unwrap_or_default().trim();
                    // The id the transcript's history gives the same block.
                    let id = format!("{msg}:{uuid}:{i}");
                    let taken = self.taken.entry(msg.clone()).or_default();
                    let replaces = self.streamed.get(&msg).and_then(|ids| ids.get(*taken)).cloned();
                    if replaces.is_some() {
                        *taken += 1;
                    }
                    if text.is_empty() && replaces.is_none() {
                        continue;
                    }
                    let mut row = sessions::row(&id, "claude", &summary::cut(text, 20_000));
                    if let Some(r) = replaces {
                        row["replaces"] = Value::from(r);
                    }
                    out.push(Out::Row(row));
                }
                Some("tool_use") => {
                    let id = b["id"].as_str().unwrap_or_default();
                    let shown = summary::shown(b["name"].as_str().unwrap_or_default(), &b["input"], &self.cwd);
                    let row = sessions::tool_row(id, &shown, "run");
                    self.rows.put(row.clone());
                    out.push(Out::Row(row));
                }
                _ => {}
            }
        }
        out
    }

    fn user(&mut self, v: &Value) -> Vec<Out> {
        let mut out = Vec::new();
        for b in v["message"]["content"].as_array().into_iter().flatten() {
            if b["type"] != "tool_result" {
                continue;
            }
            let id = b["tool_use_id"].as_str().unwrap_or_default();
            let result = summary::result_text(&b["content"]);
            let denied = self.denied.contains(id);
            if let Some(row) = self.rows.get_mut(id) {
                sessions::finish_tool(row, b["is_error"] == true, &result, denied);
                out.push(Out::Row(row.clone()));
            }
        }
        out
    }

    fn result(&mut self, v: &Value) -> Vec<Out> {
        self.done = true;
        let ok = v["subtype"] == "success" && v["is_error"] != true;
        let words = match v["subtype"].as_str() {
            _ if ok => String::new(),
            Some("error_max_turns") => "stopped: it reached its limit of turns".into(),
            Some("error_max_budget_usd") => "stopped: it reached its spending limit".into(),
            _ => {
                let errors: Vec<&str> = v["errors"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                let said = if errors.is_empty() { v["result"].as_str().unwrap_or("").to_owned() } else { errors.join(" · ") };
                if said.trim().is_empty() {
                    "Claude could not finish".into()
                } else {
                    summary::cut(&summary::one_line(&said), 160)
                }
            }
        };
        vec![Out::Done { ok, words }]
    }
}

/// An assistant message's error, in words.
fn error_words(error: &str) -> String {
    match error {
        "authentication_failed" => "Claude Code is not signed in on the computer: run claude there and sign in",
        "billing_error" => "the account has a billing problem",
        "rate_limit" => "a usage limit was reached: try again later",
        "overloaded" | "server_error" => "Claude is busy: try again shortly",
        "max_output_tokens" => "the answer ran past its length limit",
        _ => "Claude could not answer",
    }
    .to_owned()
}

/// The rows a turn's whole stream gives, played at once (the tests).
pub fn play(cwd: &str, lines: &str) -> Vec<Out> {
    let mut t = Turn::new(cwd);
    lines.lines().flat_map(|l| t.line(l)).collect()
}

/// A note row.
pub fn note(id: &str, words: &str) -> Value {
    json!({"id": id, "k": "note", "text": words})
}
