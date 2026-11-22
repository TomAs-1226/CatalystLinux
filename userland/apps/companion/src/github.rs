//! GitHub's token for the handheld's Git, three ways:
//!
//! - **This computer's sign-in** (asked first, every time): `gh auth token`
//!   if GitHub CLI is signed in, else Git Credential Manager's credential
//!   for github.com (`git credential fill`, which may show its own sign-in).
//! - **Sign in with a code**: GitHub's device flow, when this build or this
//!   computer has an OAuth app's client ID (`CATALYST_GITHUB_CLIENT_ID`,
//!   or a `github-client-id` file beside the program or in its settings).
//! - **A token pasted.**
//!
//! Each is checked with GitHub (whose it is) before it goes to the handheld.
//! Tokens go to programs on their standard input, never their command line.

use std::io::Write;
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;

use crate::win;

/// `program args`, with `input` on its standard input; its output if it worked.
fn run(program: &str, args: &[&str], input: &str, envs: &[(&str, &str)]) -> Option<String> {
    let mut child = win::command(program).args(args).envs(envs.iter().copied()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    child.stdin.take()?.write_all(input.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The GitHub sign-in this computer already has, and where it came from.
pub fn from_this_computer() -> Result<(String, &'static str), String> {
    if let Some(token) = run("gh", &["auth", "token", "--hostname", "github.com"], "", &[]).map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()) {
        return Ok((token, "GitHub CLI"));
    }
    let asked = "protocol=https\nhost=github.com\n\n";
    if let Some(out) = run("git", &["credential", "fill"], asked, &[("GIT_TERMINAL_PROMPT", "0")]) {
        if let Some(token) = out.lines().find_map(|l| l.strip_prefix("password=")).map(str::trim).filter(|t| !t.is_empty()) {
            return Ok((token.to_owned(), "Git Credential Manager"));
        }
    }
    Err("this computer is not signed in to GitHub (GitHub CLI or Git Credential Manager)".into())
}

/// Whose token it is (their login), as GitHub says.
pub fn whose(token: &str) -> Result<String, String> {
    let header = format!("Authorization: Bearer {token}\nAccept: application/vnd.github+json\nUser-Agent: catalyst-companion\n");
    let out = run("curl", &["-sS", "--max-time", "20", "-H", "@-", "https://api.github.com/user"], &header, &[]).ok_or("could not reach GitHub")?;
    let v: Value = serde_json::from_str(&out).map_err(|_| "GitHub's answer was not one")?;
    v["login"].as_str().map(str::to_owned).ok_or_else(|| match v["message"].as_str() {
        Some("Bad credentials") => "GitHub does not know that token".to_owned(),
        Some(m) => format!("GitHub said: {m}"),
        None => "GitHub did not say whose it is".to_owned(),
    })
}

/// The OAuth app's client ID for signing in with a code, if there is one.
pub fn client_id() -> Option<String> {
    static FOUND: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    FOUND.get_or_init(look_for_client_id).clone()
}

fn look_for_client_id() -> Option<String> {
    let valid = |s: String| Some(s.trim().to_owned()).filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_'));
    std::env::var("CATALYST_GITHUB_CLIENT_ID")
        .ok()
        .and_then(valid)
        .or_else(|| option_env!("CATALYST_GITHUB_CLIENT_ID").map(str::to_owned).and_then(valid))
        .or_else(|| std::env::current_exe().ok().and_then(|e| std::fs::read_to_string(e.with_file_name("github-client-id")).ok()).and_then(valid))
        .or_else(|| std::fs::read_to_string(win::app_dir().join("github-client-id")).ok().and_then(valid))
}

/// A device-flow sign-in under way.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub user_code: String,
    pub verification_uri: String,
    device_code: String,
    pub interval: Duration,
    pub expires: std::time::Instant,
}

fn post_form(url: &str, form: &str) -> Result<Value, String> {
    let out = run("curl", &["-sS", "--max-time", "20", "-X", "POST", "-H", "Accept: application/json", "--data-binary", "@-", url], form, &[]).ok_or("could not reach GitHub")?;
    serde_json::from_str(&out).map_err(|_| "GitHub's answer was not one".to_owned())
}

pub fn device_start(client_id: &str) -> Result<Device, String> {
    let v = post_form("https://github.com/login/device/code", &format!("client_id={client_id}&scope=repo"))?;
    Ok(Device {
        user_code: v["user_code"].as_str().ok_or_else(|| v["error_description"].as_str().unwrap_or("GitHub gave no code").to_owned())?.to_owned(),
        verification_uri: v["verification_uri"].as_str().unwrap_or("https://github.com/login/device").to_owned(),
        device_code: v["device_code"].as_str().unwrap_or_default().to_owned(),
        interval: Duration::from_secs(v["interval"].as_u64().unwrap_or(5).max(5)),
        expires: std::time::Instant::now() + Duration::from_secs(v["expires_in"].as_u64().unwrap_or(900)),
    })
}

/// The token once the code has been entered on github.com; `None` while
/// GitHub is still waiting for it.
pub fn device_poll(client_id: &str, device: &mut Device) -> Result<Option<String>, String> {
    let form = format!("client_id={client_id}&device_code={}&grant_type=urn:ietf:params:oauth:grant-type:device_code", device.device_code);
    let v = post_form("https://github.com/login/oauth/access_token", &form)?;
    if let Some(token) = v["access_token"].as_str() {
        return Ok(Some(token.to_owned()));
    }
    match v["error"].as_str() {
        Some("authorization_pending") => Ok(None),
        Some("slow_down") => {
            device.interval += Duration::from_secs(5);
            Ok(None)
        }
        Some("expired_token") => Err("the code ran out: start again".into()),
        Some("access_denied") => Err("the sign-in was turned down on GitHub".into()),
        _ => Err(v["error_description"].as_str().unwrap_or("GitHub did not sign in").to_owned()),
    }
}
