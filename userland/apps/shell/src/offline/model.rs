//! The small model that words offline answers (and sorts a question the
//! rules cannot): llama.cpp's llama-server on this handheld, as Settings ›
//! On-device model runs it (`crate::local`), or one at `CATALYST_LOCAL_URL`
//! (the simulator, the evaluation on a PC).
//!
//! How much it is trusted with follows its size: from 135M parameters it
//! words two or three facts; from about 300M it also sorts questions the
//! rules cannot; from 1B it is given a few more facts. Whatever its size,
//! it has a deadline, and past it the answer made from the facts is shown.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

pub trait Model: Send + Sync {
    /// As people know it: "SmolLM2 135M".
    fn name(&self) -> String;
    /// One OpenAI-style chat completion, given at most `seconds`.
    fn chat(&self, body: &Value, seconds: u32) -> Result<Value, String>;
}

/// The model Settings › On-device model downloaded, started if it is not running.
pub struct OnDevice;

impl Model for OnDevice {
    fn name(&self) -> String {
        crate::local::model_name().to_owned()
    }

    fn chat(&self, body: &Value, seconds: u32) -> Result<Value, String> {
        crate::ai::request_local_within(body, seconds)
    }
}

/// A llama-server elsewhere, by its OpenAI base URL ("http://127.0.0.1:8091/v1").
pub struct At {
    pub url: String,
    pub name: String,
}

impl Model for At {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn chat(&self, body: &Value, seconds: u32) -> Result<Value, String> {
        let url = format!("{}/chat/completions", self.url.trim_end_matches('/'));
        let max_time = seconds.to_string();
        let mut child = Command::new("curl")
            .args(["-sS", "--max-time", &max_time, "-H", "Content-Type: application/json", "--data-binary", "@-", "-w", "\n%{http_code}", &url])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| "curl could not start".to_owned())?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(body.to_string().as_bytes()).map_err(|_| "the question could not be sent".to_owned())?;
        }
        let output = child.wait_with_output().map_err(|_| "the model stopped".to_owned())?;
        if !output.status.success() {
            return Err(if output.status.code() == Some(28) { "the model took too long".into() } else { "the model did not answer".into() });
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
        if code.trim() != "200" {
            return Err(format!("the model answered {}", code.trim()));
        }
        serde_json::from_str(body).map_err(|_| "the answer could not be read".to_owned())
    }
}

/// The model offline answers use, if there is one: one named by
/// `CATALYST_LOCAL_URL` (and `CATALYST_LOCAL_MODEL`), else the on-device
/// model once downloaded (not in the simulator, whose scripted service has
/// no such model).
pub fn find() -> Option<Box<dyn Model>> {
    if let Some(url) = std::env::var("CATALYST_LOCAL_URL").ok().filter(|u| !u.trim().is_empty()) {
        let name = std::env::var("CATALYST_LOCAL_MODEL").or_else(|_| std::env::var("CATALYST_EVAL_MODEL")).unwrap_or_else(|_| "the model".into());
        return Some(Box::new(At { url, name }));
    }
    if crate::ai::demo() || !crate::local::ready() {
        return None;
    }
    Some(Box::new(OnDevice))
}

/// Its size in millions of parameters, from its name: "SmolLM2 135M" 135,
/// "Qwen3-0.6B-Q4_K_M" 600, "gemma-3-1b-it" 1000. Unknown: 135.
pub fn size_m(name: &str) -> u32 {
    for part in name.split(|c: char| c == ' ' || c == '-' || c == '_') {
        let lower = part.to_ascii_lowercase();
        let (num, scale) = if let Some(n) = lower.strip_suffix('m') {
            (n, 1.0)
        } else if let Some(n) = lower.strip_suffix('b') {
            (n, 1000.0)
        } else {
            continue;
        };
        if let Ok(v) = num.parse::<f64>() {
            if v > 0.0 && v * scale >= 50.0 {
                return (v * scale).round() as u32;
            }
        }
    }
    135
}

/// What a model of a size is trusted with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tier {
    /// Facts it is given to word.
    pub lines: usize,
    /// It sorts the questions the rules cannot.
    pub route: bool,
    pub max_tokens: u32,
    /// How long it may take to word an answer.
    pub seconds: u32,
}

pub fn tier(size_m: u32) -> Tier {
    match size_m {
        0..=250 => Tier { lines: 3, route: false, max_tokens: 80, seconds: 30 },
        251..=800 => Tier { lines: 4, route: true, max_tokens: 96, seconds: 45 },
        _ => Tier { lines: 5, route: true, max_tokens: 110, seconds: 60 },
    }
}

/// A chat request: the instructions, the turns, and a grammar to hold the
/// answer to, if any. Thinking is turned off for the models that do it.
pub fn body(system: &str, turns: &[(&str, &str)], max_tokens: u32, grammar: Option<&str>) -> Value {
    let mut messages = vec![json!({"role": "system", "content": system})];
    messages.extend(turns.iter().map(|(role, text)| json!({"role": role, "content": text})));
    let mut body = json!({
        "messages": messages,
        "max_tokens": max_tokens,
        "temperature": 0.1,
        "top_k": 20,
        "top_p": 0.9,
        "stream": false,
        "cache_prompt": true,
        "chat_template_kwargs": {"enable_thinking": false},
    });
    if let Some(g) = grammar {
        body["grammar"] = json!(g);
    }
    body
}

/// The words of a reply, and the tokens it took: (text, prompt, answer).
pub fn reply(v: &Value) -> Result<(String, u32, u32), String> {
    if let Some(message) = v["error"]["message"].as_str() {
        return Err(message.to_owned());
    }
    let text = v["choices"][0]["message"]["content"].as_str().ok_or("the model gave no answer")?.to_owned();
    let prompt = v["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32;
    let answer = v["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32;
    Ok((text, prompt, answer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_is_sized_from_its_name() {
        assert_eq!(size_m("SmolLM2 135M"), 135);
        assert_eq!(size_m("SmolLM2-360M-Instruct-Q4_K_M"), 360);
        assert_eq!(size_m("Qwen3-0.6B-Q4_K_M"), 600);
        assert_eq!(size_m("gemma-3-1b-it-Q4_K_M"), 1000);
        assert_eq!(size_m("LFM2.5-1.2B-Instruct-Q4_K_M"), 1200);
        assert_eq!(size_m("gemma-3-270m-it-Q8_0"), 270);
        assert_eq!(size_m("mystery"), 135);
        assert!(!tier(135).route && tier(360).route && tier(1200).lines > tier(360).lines);
        let b = body("sys", &[("user", "q")], 50, Some("root ::= \"a\""));
        assert_eq!(b["grammar"], "root ::= \"a\"");
        assert_eq!(b["chat_template_kwargs"]["enable_thinking"], false);
        let (text, p, a) = reply(&json!({"choices": [{"message": {"content": "hi"}}], "usage": {"prompt_tokens": 12, "completion_tokens": 3}})).unwrap();
        assert_eq!((text.as_str(), p, a), ("hi", 12, 3));
    }
}
