//! Finding the newest release and fetching its files, through `curl` (which
//! the image already has, with its certificates): no TLS in this program.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

/// A release found: the URL of each of its files, by name.
pub struct Release {
    pub tag: String,
    pub files: Vec<(String, String)>,
}

impl Release {
    pub fn url(&self, name: &str) -> Option<&str> {
        self.files.iter().find(|(n, _)| n == name).map(|(_, u)| u.as_str())
    }
}

const AGENT: &str = "catalyst-update";

fn curl() -> Command {
    let mut c = Command::new("curl");
    c.args(["-fsSL", "--retry", "3", "--connect-timeout", "20", "-A", AGENT]);
    c
}

/// Fetch a URL's body.
pub fn get(url: &str) -> Result<Vec<u8>, String> {
    let out = curl().arg(url).output().map_err(|e| format!("could not run curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("could not fetch {url}: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout)
}

/// Download a URL to a file, carrying on from wherever an earlier download
/// stopped. `progress` is told the bytes so far, now and then.
pub fn download(url: &str, to: &Path, size: u64, progress: &mut dyn FnMut(u64)) -> Result<(), String> {
    if std::fs::metadata(to).is_ok_and(|m| m.len() == size) {
        return Ok(());
    }
    if std::fs::metadata(to).is_ok_and(|m| m.len() > size) {
        std::fs::remove_file(to).ok();
    }
    let mut child = curl().args(["-C", "-", "-o"]).arg(to).arg(url).spawn().map_err(|e| format!("could not run curl: {e}"))?;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                progress(std::fs::metadata(to).map_or(0, |m| m.len()));
                // 33: the file was already whole, so there was nothing to resume.
                return if status.success() || status.code() == Some(33) { Ok(()) } else { Err(format!("the download stopped ({status})")) };
            }
            Ok(None) => {
                progress(std::fs::metadata(to).map_or(0, |m| m.len()));
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
            Err(e) => return Err(format!("the download: {e}")),
        }
    }
}

/// The newest release on a channel. `source` is `github:owner/repo`
/// (releases on GitHub: a stable one is a full release, a nightly one a
/// pre-release) or `url:<base>` (a folder holding one release's files).
pub fn latest(source: &str, channel: &str) -> Result<Release, String> {
    if let Some(base) = source.strip_prefix("url:") {
        let base = base.trim_end_matches('/');
        let names = ["manifest.json", "manifest.sig"];
        // A plain folder lists nothing: every file is asked for by name.
        return Ok(Release { tag: "url".into(), files: names.iter().map(|n| (n.to_string(), format!("{base}/{n}"))).chain(std::iter::once(("*".into(), format!("{base}/")))).collect() });
    }
    let repo = source.strip_prefix("github:").ok_or_else(|| format!("no update source set ({source:?})"))?;
    let body = get(&format!("https://api.github.com/repos/{repo}/releases?per_page=20"))?;
    let all: Value = serde_json::from_slice(&body).map_err(|e| format!("GitHub's answer is not JSON: {e}"))?;
    let want_pre = channel == "nightly";
    let release = all
        .as_array()
        .into_iter()
        .flatten()
        .find(|r| r["draft"].as_bool() == Some(false) && r["prerelease"].as_bool() == Some(want_pre) && r["assets"].as_array().is_some_and(|a| a.iter().any(|f| f["name"] == "manifest.json")))
        .ok_or_else(|| format!("no {channel} release yet"))?;
    let files = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| Some((a["name"].as_str()?.to_owned(), a["browser_download_url"].as_str()?.to_owned())))
        .collect();
    Ok(Release { tag: release["tag_name"].as_str().unwrap_or_default().to_owned(), files })
}

/// A file of a release by name: listed, or (a plain folder) under its base.
pub fn file_url(release: &Release, name: &str) -> Option<String> {
    release.url(name).map(str::to_owned).or_else(|| release.url("*").map(|base| format!("{base}{name}")))
}
