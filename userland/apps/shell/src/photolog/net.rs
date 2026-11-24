//! Requests to Anthropic and BeaconDB, with curl, as the rest of the shell
//! makes them, in the background's manners (nice, killed first, the card
//! served last).
//!
//! Secrets never touch a command line or a file: every header, the key's
//! among them, reaches curl on its standard input (`-H @-`). Only the body
//! goes through a file (it can be a few megabytes of photos), written where
//! only root reads and removed as soon as curl has it. Nothing is logged.

use std::io::Write;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};

/// What a server answered: its status and body.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub code: u16,
    pub body: String,
}

/// How long one request may take.
const SECONDS: u32 = 120;

/// One request. `headers` are "Name: value" lines; `url` must be HTTPS,
/// except to this computer (the tests' stand-in servers).
pub fn call(method: &str, url: &str, headers: &str, body: Option<&[u8]>, agent: &str) -> Result<Reply, String> {
    let local = url.starts_with("http://127.0.0.1:");
    if !url.starts_with("https://") && !local {
        return Err("only https".into());
    }
    let file = match body {
        Some(bytes) => {
            static N: AtomicU64 = AtomicU64::new(0);
            let dir = super::dir().join("out");
            std::fs::create_dir_all(&dir).map_err(|_| "the request could not be prepared")?;
            let f = dir.join(format!("{}-{}.json", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
            crate::sys::write_private(&f, bytes).map_err(|_| "the request could not be prepared")?;
            Some(f)
        }
        None => None,
    };
    let result = run(method, url, headers, file.as_deref(), agent, local);
    if let Some(f) = &file {
        std::fs::remove_file(f).ok();
    }
    result
}

fn run(method: &str, url: &str, headers: &str, file: Option<&std::path::Path>, agent: &str, local: bool) -> Result<Reply, String> {
    let mut cmd = crate::tools::quiet("curl");
    cmd.args(["-sS", "--max-time", &SECONDS.to_string(), "-X", method, "-A", agent, "-w", "\n%{http_code}", "-H", "@-"]);
    cmd.args(["--proto", if local { "=http" } else { "=https" }]);
    if let Some(f) = file {
        cmd.arg("--data-binary").arg(format!("@{}", f.display()));
    }
    cmd.arg(url);
    crate::music::polite::polite(&mut cmd, crate::music::polite::Manner::Background);
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|_| "curl could not start".to_owned())?;
    if let Some(mut stdin) = child.stdin.take() {
        // Ends with the headers: curl reads to the end of its input.
        stdin.write_all(headers.as_bytes()).map_err(|_| "the request could not be sent".to_owned())?;
    }
    let out = child.wait_with_output().map_err(|_| "the request stopped".to_owned())?;
    if !out.status.success() {
        return Err(match out.status.code() {
            Some(6) => "no internet".into(),
            Some(7) => "could not reach it".into(),
            Some(28) => "no answer in time".into(),
            Some(35 | 60) => "the secure connection failed · is the clock right?".into(),
            _ => "the request failed".into(),
        });
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    Ok(Reply { code: code.trim().parse().unwrap_or(0), body: body.to_owned() })
}
