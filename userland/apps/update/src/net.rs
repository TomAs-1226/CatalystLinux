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

/// The certificates servers are checked against. Named outright rather than
/// left to curl's built-in default: a curl built without one (as on mbedTLS
/// before 0.4.2) trusts nothing, and then no update could ever be fetched.
const CA_BUNDLE: &str = "/etc/ssl/certs/ca-certificates.crt";

fn curl() -> Command {
    let mut c = Command::new("curl");
    c.args(["-fsSL", "--retry", "3", "--connect-timeout", "20", "-A", AGENT]);
    if std::path::Path::new(CA_BUNDLE).exists() {
        c.args(["--cacert", CA_BUNDLE]);
    }
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
///
/// A connection that drops part-way (Wi-Fi blinking, a server closing early:
/// curl's "end of response with … bytes missing") is picked up again from
/// where it stopped, for as long as each try gets further; only one that
/// makes no headway STALLS times running is given up on.
pub fn download(url: &str, to: &Path, size: u64, progress: &mut dyn FnMut(u64)) -> Result<(), String> {
    const STALLS: u32 = 6;
    let have = |to: &Path| std::fs::metadata(to).map_or(0, |m| m.len());
    if have(to) > size {
        std::fs::remove_file(to).ok();
    }
    let mut stalls = 0;
    let mut last = None;
    loop {
        if have(to) == size {
            progress(size);
            return Ok(());
        }
        let before = have(to);
        let status = run_once(url, to, &mut *progress)?;
        // 33: the file was already whole, so there was nothing to resume.
        if (status.success() || status.code() == Some(33)) && have(to) == size {
            progress(size);
            return Ok(());
        }
        if have(to) > size {
            std::fs::remove_file(to).ok();
        }
        stalls = if have(to) > before { 0 } else { stalls + 1 };
        last = Some(status);
        if stalls >= STALLS {
            return Err(format!("the download stopped ({})", last.map_or_else(String::new, |s| s.to_string())));
        }
        std::thread::sleep(std::time::Duration::from_secs(2 + 3 * u64::from(stalls)));
    }
}

/// One go at a download with curl, resuming the file as it stands.
fn run_once(url: &str, to: &Path, progress: &mut dyn FnMut(u64)) -> Result<std::process::ExitStatus, String> {
    let mut child = curl().args(["-C", "-", "-o"]).arg(to).arg(url).spawn().map_err(|e| format!("could not run curl: {e}"))?;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                progress(std::fs::metadata(to).map_or(0, |m| m.len()));
                return Ok(status);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_cut_short_carries_on_from_where_it_stopped() {
        if !std::process::Command::new("curl").arg("--version").output().is_ok_and(|o| o.status.success()) {
            eprintln!("no curl here: skipped");
            return;
        }
        let dir = std::env::temp_dir().join(format!("catalyst-download-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let whole: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let source = dir.join("source.bin");
        std::fs::write(&source, &whole).unwrap();
        // What an earlier try got before the connection dropped.
        let to = dir.join("got.bin");
        std::fs::write(&to, &whole[..120_000]).unwrap();
        let url = format!("file:///{}", source.to_string_lossy().replace('\\', "/").trim_start_matches('/'));
        let mut seen = Vec::new();
        download(&url, &to, whole.len() as u64, &mut |n| seen.push(n)).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), whole, "picked up where it stopped, whole and in order");
        assert_eq!(seen.last(), Some(&(whole.len() as u64)));
        // Whole already: nothing to fetch.
        download(&url, &to, whole.len() as u64, &mut |_| {}).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }
}
