//! YouTube Music's sign-in, in one click.
//!
//! The app starts itself again as a small window (`--youtube-sign-in`): a
//! WebView2 browser on Google's sign-in for YouTube Music, with a profile of
//! its own (`%LOCALAPPDATA%\Catalyst Companion\YouTube Music`, nothing
//! shared with Edge or any other browser). Once music.youtube.com is open
//! and signed in, the window reads that profile's cookies for it through
//! WebView2's cookie manager (HTTP-only ones too), writes them to the app
//! down a pipe as a Netscape cookies.txt, and closes, before YouTube has
//! reason to renew them. The app sends them to the handheld, which checks
//! and keeps them as its setup page does. They are never written anywhere
//! else on this computer and never logged. Sign out deletes the profile.

use std::io::Read;
use std::process::Stdio;

use crate::win;

/// Where the window opens: Google's sign-in, coming back to YouTube Music.
const SIGN_IN: &str = "https://accounts.google.com/ServiceLogin?ltmpl=music&service=youtube&passive=true&continue=https%3A%2F%2Fwww.youtube.com%2Fsignin%3Faction_handle_signin%3Dtrue%26next%3Dhttps%253A%252F%252Fmusic.youtube.com%252F";
const MUSIC: &str = "https://music.youtube.com/";

/// One cookie, as the browser holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Cookie {
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    /// Unix seconds; 0 for one that lasts the session.
    pub expires: i64,
    pub name: String,
    pub value: String,
}

/// Signed in: the cookie YouTube's requests are signed with, and one that
/// says who is signed in (as the handheld's own check has it).
pub fn signed_in(cookies: &[Cookie]) -> bool {
    let has = |names: &[&str]| cookies.iter().any(|c| names.contains(&c.name.as_str()) && !c.value.is_empty() && c.domain.trim_start_matches('.').ends_with("youtube.com"));
    has(&["SAPISID", "__Secure-3PAPISID"]) && has(&["SID", "__Secure-1PSID", "__Secure-3PSID"])
}

/// YouTube's cookies as a Netscape cookies.txt (curl's `#HttpOnly_` mark
/// on those scripts cannot read), which the handheld takes.
pub fn netscape(cookies: &[Cookie]) -> String {
    let mut out = String::from("# Netscape HTTP Cookie File\n");
    for c in cookies.iter().filter(|c| c.domain.trim_start_matches('.').ends_with("youtube.com") && !c.name.is_empty()) {
        // Every one for the whole of youtube.com, as Google sets its own.
        let domain = format!(".{}", c.domain.trim_start_matches('.'));
        let flag = |b: bool| if b { "TRUE" } else { "FALSE" };
        let value: String = c.value.chars().filter(|ch| !ch.is_control()).collect();
        out.push_str(&format!("{}{domain}\tTRUE\t{}\t{}\t{}\t{}\t{value}\n", if c.http_only { "#HttpOnly_" } else { "" }, if c.path.is_empty() { "/" } else { &c.path }, flag(c.secure), c.expires.max(0), c.name));
    }
    out
}

/// The sign-in window, run, and its cookies.txt once signed in.
pub fn sign_in() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = win::command(&exe.to_string_lossy()).arg("--youtube-sign-in").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("could not open the sign-in window: {e}"))?;
    let mut jar = String::new();
    child.stdout.take().ok_or("no pipe from the sign-in window")?.read_to_string(&mut jar).map_err(|e| e.to_string())?;
    let mut why = String::new();
    if let Some(mut e) = child.stderr.take() {
        e.read_to_string(&mut why).ok();
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    match status.code() {
        Some(0) if !jar.is_empty() => Ok(jar),
        Some(2) => Err("the sign-in window was closed before signing in".into()),
        _ => Err(why.lines().last().map_or_else(|| "the sign-in window stopped".to_owned(), |l| l.trim().to_owned())),
    }
}

/// This computer's sign-in forgotten: the window's whole profile deleted.
pub fn forget_here() -> Result<(), String> {
    // WebView2's own processes let go of the profile a moment after the window closes.
    let mut last = None;
    for _ in 0..10 {
        match std::fs::remove_dir_all(win::webview_dir()) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
    Err(format!("could not clear this computer's sign-in: {}", last.map_or_else(String::new, |e| e.to_string())))
}

/// Whether this computer still holds a sign-in (the profile is there).
pub fn held_here() -> bool {
    win::webview_dir().exists()
}

/// `--youtube-sign-in`: the window itself. Exits 0 having written the
/// cookies.txt, 2 if closed first, 1 if WebView2 could not start.
#[cfg(windows)]
pub fn window() -> ! {
    use std::time::{Duration, Instant};
    use tao::dpi::LogicalSize;
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoop};
    use tao::window::WindowBuilder;
    use wry::{WebContext, WebViewBuilder};

    let fail = |why: String| -> ! {
        eprintln!("{why}");
        std::process::exit(1)
    };
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new().with_title("Sign in to YouTube Music · Catalyst Companion").with_inner_size(LogicalSize::new(520.0, 780.0)).with_min_inner_size(LogicalSize::new(420.0, 560.0)).build(&event_loop).unwrap_or_else(|e| fail(format!("could not open the window: {e}")));
    std::fs::create_dir_all(win::webview_dir()).ok();
    let mut context = WebContext::new(Some(win::webview_dir()));
    // Trying the cookie manager with no account: YouTube Music opened
    // signed out, and the names of the cookies it set (never their values)
    // said, HTTP-only ones marked.
    let probe = std::env::var_os("CATALYST_COMPANION_COOKIE_PROBE").is_some();
    let webview = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(if probe { MUSIC } else { SIGN_IN })
        .with_devtools(false)
        .build(&window)
        .unwrap_or_else(|e| fail(format!("WebView2 would not start ({e}): install the WebView2 Runtime from Microsoft")));
    let started = Instant::now();
    let mut nudged = false;
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(700));
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => std::process::exit(2),
            Event::NewEvents(_) => {
                let url = webview.url().unwrap_or_default();
                let on_music = url.starts_with(MUSIC);
                // Signed in already (the profile remembers): Google sends it straight on.
                if !on_music && !nudged && started.elapsed() > Duration::from_secs(20) && url.contains("youtube.com") {
                    nudged = true;
                    webview.load_url(MUSIC).ok();
                }
                if !on_music {
                    return;
                }
                let Ok(found) = webview.cookies_for_url(MUSIC) else { return };
                let cookies: Vec<Cookie> = found
                    .iter()
                    .map(|c| Cookie {
                        domain: c.domain().unwrap_or("youtube.com").to_owned(),
                        path: c.path().unwrap_or("/").to_owned(),
                        secure: c.secure().unwrap_or(false),
                        http_only: c.http_only().unwrap_or(false),
                        expires: c.expires_datetime().map_or(0, |t| t.unix_timestamp()),
                        name: c.name().to_owned(),
                        value: c.value().to_owned(),
                    })
                    .collect();
                if probe && started.elapsed() > Duration::from_secs(6) {
                    let names: Vec<String> = cookies.iter().map(|c| format!("{}{} ({})", c.name, if c.http_only { " [http-only]" } else { "" }, c.domain)).collect();
                    eprintln!("probe: {} cookies for music.youtube.com, signed in {}: {}", cookies.len(), signed_in(&cookies), names.join(", "));
                    eprintln!("probe: as cookies.txt, {} lines", netscape(&cookies).lines().count());
                    std::process::exit(3);
                }
                if signed_in(&cookies) {
                    use std::io::Write;
                    let mut out = std::io::stdout().lock();
                    out.write_all(netscape(&cookies).as_bytes()).ok();
                    out.flush().ok();
                    std::process::exit(0);
                }
            }
            _ => {}
        }
    })
}

#[cfg(not(windows))]
pub fn window() -> ! {
    eprintln!("signing in to YouTube Music needs Windows' WebView2");
    std::process::exit(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie(name: &str, value: &str, http_only: bool) -> Cookie {
        Cookie { domain: ".youtube.com".into(), path: "/".into(), secure: true, http_only, expires: 1_900_000_000, name: name.into(), value: value.into() }
    }

    #[test]
    fn a_signed_in_jar_is_a_netscape_cookies_txt_with_http_only_marked() {
        let jar = vec![cookie("SAPISID", "sap", false), cookie("__Secure-3PSID", "sid", true), cookie("PREF", "f6=40", false), Cookie { domain: "accounts.google.com".into(), ..cookie("GAPS", "g", true) }];
        assert!(signed_in(&jar));
        let text = netscape(&jar);
        assert!(text.starts_with("# Netscape HTTP Cookie File\n"));
        assert!(text.contains(".youtube.com\tTRUE\t/\tTRUE\t1900000000\tSAPISID\tsap\n"));
        assert!(text.contains("#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t1900000000\t__Secure-3PSID\tsid\n"), "HTTP-only ones kept and marked");
        assert!(!text.contains("GAPS"), "only YouTube's go to the handheld");
    }

    #[test]
    fn not_signed_in_until_both_cookies_are_there() {
        assert!(!signed_in(&[cookie("SAPISID", "sap", false)]));
        assert!(!signed_in(&[cookie("SID", "sid", true)]));
        assert!(!signed_in(&[cookie("SAPISID", "", false), cookie("SID", "sid", true)]));
        let session = Cookie { expires: 0, ..cookie("YSC", "y", true) };
        assert!(netscape(&[session]).contains("\t0\tYSC\ty"), "a session cookie has no expiry");
    }
}
