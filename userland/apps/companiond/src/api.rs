//! The requests a paired computer makes, and pairing itself.
//!
//! Only `GET /v1/hello` and the two pairing requests are answered without a
//! token; everything else needs `Authorization: Bearer <token>` of a
//! computer paired here, and a wrong one waits a moment before it is told.

use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use catalyst_companion_proto::{self as proto, pair};
use serde_json::{json, Value};

use crate::files;
use crate::http::{encode, Body, Request, Response};
use crate::shell::Shell;
use crate::store::{self, Computer, Paths};

/// The most a JSON request may be.
const JSON_MOST: u64 = 64 * 1024;
/// A cookies.txt for YouTube Music is a few KB; the shell takes 64 KB.
const COOKIES_MOST: u64 = 64 * 1024;
/// A song, at most.
const SONG_MOST: u64 = 2 << 30;
/// How long the computer has between the two halves of pairing.
const PAIR_SESSION: Duration = Duration::from_secs(60);

/// The fields `POST /v1/settings` may carry: those of the page for setting
/// up from a computer (remote.rs), but YouTube Music's cookies, which have
/// their own request.
const SETTINGS_FIELDS: [&str; 20] = ["team", "ssid1", "psk1", "ssid2", "psk2", "tba", "nexus", "openai", "claude", "github", "ssh_key", "accent", "mode", "text", "home", "choices", "open_robot", "match_report", "ssh", "calm"];

struct Pending {
    session: String,
    key: Vec<u8>,
    name: String,
    at: Instant,
}

#[derive(Default)]
pub struct PairState {
    /// The code tried at, and how many times.
    code: String,
    tries: u32,
    pending: Option<Pending>,
}

pub struct Ctx {
    pub paths: Paths,
    pub shell: Box<dyn Shell>,
    pub pairing: Mutex<PairState>,
    /// On the handheld: the updater is there, the shell is a service, and a
    /// restore restarts it all.
    pub on_device: bool,
    pub updater: PathBuf,
    /// How long a wrong token or code waits (the tests wait for nothing).
    pub penalty: Duration,
}

fn read_json(body: &mut dyn Read, len: u64) -> Result<Value, Response> {
    if len > JSON_MOST {
        return Err(Response::error(413, "that is more than a request takes"));
    }
    let mut text = String::new();
    body.take(len).read_to_string(&mut text).map_err(|_| Response::error(400, "the request did not arrive whole"))?;
    serde_json::from_str(&text).map_err(|_| Response::error(400, "that was not JSON"))
}

fn shell_reply(r: Result<String, String>) -> Response {
    match r {
        Ok(text) => Response::json(200, serde_json::from_str(&text).unwrap_or(json!({"ok": true}))),
        Err(why) if why.contains("not running") => Response::error(503, why),
        Err(why) => Response::error(422, why),
    }
}

impl Ctx {
    fn hello(&self, paired: bool) -> Value {
        json!({"catalyst": 1, "id": store::id(&self.paths), "name": store::name(), "version": store::version(), "paired": paired})
    }

    /// `POST /v1/pair`: SPAKE2's first message in, the handheld's answer
    /// and proof out.
    fn pair_start(&self, req: &Request, body: &mut dyn Read, fingerprint: &[u8]) -> Response {
        let v = match read_json(body, req.length) {
            Ok(v) => v,
            Err(r) => return r,
        };
        let Some(code) = store::pairing_code(&self.paths) else {
            return Response::error(410, "the handheld is not pairing: on it, open Settings › Computer companion › Pair a computer");
        };
        let Some(message) = v["message"].as_str().and_then(proto::unhex) else {
            return Response::error(400, "no pairing message");
        };
        let name: String = v["name"].as_str().unwrap_or("a computer").chars().filter(|c| !c.is_control()).take(40).collect();
        let mut state = self.pairing.lock().unwrap_or_else(|e| e.into_inner());
        if state.code != code {
            *state = PairState { code: code.clone(), ..PairState::default() };
        }
        // Each try is one guess at the code, whether or not it is finished.
        state.tries += 1;
        if state.tries > proto::TRIES {
            drop(state);
            store::end_pairing(&self.paths, None);
            return Response::error(429, "too many tries: start pairing again on the handheld");
        }
        std::thread::sleep(self.penalty);
        let Ok((answer, key)) = pair::handheld_answer(&code, &message) else {
            return Response::error(400, "that was not a pairing message");
        };
        let session = store::random_hex(16);
        let proof = pair::confirm(&key, pair::Side::Handheld, fingerprint);
        state.pending = Some(Pending { session: session.clone(), key, name, at: Instant::now() });
        Response::json(200, json!({"session": session, "message": proto::hex(&answer), "proof": proto::hex(&proof)}))
    }

    /// `POST /v1/pair/confirm`: the computer's proof in, its token out.
    fn pair_confirm(&self, req: &Request, body: &mut dyn Read, fingerprint: &[u8]) -> Response {
        let v = match read_json(body, req.length) {
            Ok(v) => v,
            Err(r) => return r,
        };
        let mut state = self.pairing.lock().unwrap_or_else(|e| e.into_inner());
        let pending = match state.pending.take() {
            Some(p) if p.at.elapsed() < PAIR_SESSION && v["session"].as_str() == Some(p.session.as_str()) => p,
            _ => return Response::error(409, "that pairing is over: start again"),
        };
        let theirs = v["proof"].as_str().and_then(proto::unhex).unwrap_or_default();
        let ours = pair::confirm(&pending.key, pair::Side::Computer, fingerprint);
        if !proto::same(&theirs, &ours) {
            let spent = state.tries >= proto::TRIES;
            drop(state);
            if spent {
                store::end_pairing(&self.paths, None);
            }
            std::thread::sleep(self.penalty);
            return Response::error(403, "the code did not match");
        }
        *state = PairState::default();
        drop(state);
        let token = store::random_hex(32);
        let computer = Computer { id: store::random_hex(8), name: pending.name.clone(), hash: proto::token_hash(&token), paired: store::now() };
        let mut list = store::computers(&self.paths);
        list.push(computer.clone());
        if store::save_computers(&self.paths, &list).is_err() {
            return Response::error(507, "the handheld could not keep the pairing");
        }
        store::end_pairing(&self.paths, Some(&pending.name));
        let mut hello = self.hello(true);
        hello["token"] = json!(token);
        hello["computer"] = json!(computer.id);
        Response::json(200, hello)
    }

    fn settings(&self, req: &Request, body: &mut dyn Read) -> Response {
        let v = match read_json(body, req.length) {
            Ok(v) => v,
            Err(r) => return r,
        };
        let Some(fields) = v.as_object() else { return Response::error(400, "settings come as an object") };
        let mut form = Vec::new();
        for (k, value) in fields {
            if !SETTINGS_FIELDS.contains(&k.as_str()) {
                return Response::error(400, format!("{k} is not a setting this takes"));
            }
            let value = match value {
                Value::String(s) => s.clone(),
                Value::Bool(true) => "on".into(),
                Value::Bool(false) => continue,
                Value::Number(n) => n.to_string(),
                _ => continue,
            };
            form.push(format!("{}={}", encode(k), encode(&value)));
        }
        shell_reply(self.shell.ask("apply", &form.join("&")))
    }

    fn sign_in_music(&self, req: &Request, body: &mut dyn Read) -> Response {
        if req.length > COOKIES_MOST {
            return Response::error(413, "that is more than YouTube Music's sign-in would be");
        }
        let mut cookies = String::new();
        if body.take(req.length).read_to_string(&mut cookies).is_err() {
            return Response::error(400, "the sign-in did not arrive whole");
        }
        let reply = self.shell.ask("apply", &format!("cookies={}", encode(&cookies)));
        cookies.clear();
        shell_reply(reply)
    }

    fn github(&self, req: &Request, body: &mut dyn Read) -> Response {
        let v = match read_json(body, req.length) {
            Ok(v) => v,
            Err(r) => return r,
        };
        let token = v["token"].as_str().unwrap_or_default().trim();
        if token.len() < 20 || token.len() > 255 || !token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Response::error(422, "that is not a GitHub token");
        }
        shell_reply(self.shell.ask("apply", &format!("github={}", encode(token))))
    }

    fn update_status(&self) -> Value {
        let text = std::fs::read_to_string(self.paths.data.join("update/status.json")).unwrap_or_default();
        let status: Value = serde_json::from_str(&text).unwrap_or(json!({}));
        json!({"version": store::version(), "available": self.updater.exists(), "status": status})
    }

    /// A step of `catalyst-update`: a check waited for, an install or an
    /// apply left running (it belongs to no one, as the shell starts it).
    fn update_step(&self, step: &str) -> Response {
        if !self.updater.exists() {
            return Response::error(503, "this system does not update itself");
        }
        if step == "check" {
            let out = std::process::Command::new(&self.updater).arg("check").stdin(std::process::Stdio::null()).output();
            if let Some(out) = out.as_ref().ok().filter(|o| !o.status.success()) {
                let why = String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or_default().trim().trim_start_matches("catalyst-update: ").to_owned();
                return Response::error(422, if why.is_empty() { "the check did not finish".into() } else { why });
            }
            return Response::json(200, self.update_status());
        }
        let line = format!("{} {step} >/dev/null 2>&1 &", self.updater.display());
        match std::process::Command::new("sh").args(["-c", &line]).status() {
            Ok(s) if s.success() => Response::json(200, self.update_status()),
            _ => Response::error(503, format!("could not start the {step}")),
        }
    }

    fn backup(&self) -> Response {
        let list = files::backup_list(&self.paths.data);
        let len = files::backup_len(&list);
        let data = self.paths.data.clone();
        Response { status: 200, body: Body::Stream { len: Some(len), kind: "application/x-tar", write: Box::new(move |w| files::write_backup(&data, &list, w)) }, after: None }
    }

    /// The shell is stopped while /data is written under it (it would save
    /// its settings over the restored ones), and the handheld restarts after.
    fn restore(&self, body: &mut dyn Read, len: u64) -> Response {
        let service = std::path::Path::new("/etc/init.d/catalyst-shell");
        let stop = self.on_device && service.exists();
        if stop {
            std::process::Command::new(service).arg("stop").status().ok();
        }
        let result = files::restore(&self.paths.data, &mut body.take(len));
        let on_device = self.on_device;
        let after: Option<Box<dyn FnOnce() + Send>> = on_device.then(|| {
            Box::new(move || {
                std::thread::sleep(Duration::from_secs(2));
                std::process::Command::new("reboot").status().ok();
            }) as Box<dyn FnOnce() + Send>
        });
        match result {
            Ok(n) => Response { status: 200, body: Body::Json(json!({"restored": n, "restarting": on_device})), after },
            Err(why) => {
                if stop {
                    std::process::Command::new(service).arg("start").status().ok();
                }
                Response::error(422, why)
            }
        }
    }

    fn files(&self, req: &Request) -> Response {
        let (area, rel) = (req.query("area").unwrap_or_default(), req.query("path").unwrap_or_default());
        match req.path.as_str() {
            "/v1/files" => match files::list(&self.paths, area, rel) {
                Ok(v) => Response::json(200, v),
                Err(why) => Response::error(404, why),
            },
            _ => match files::resolve(&self.paths, area, rel) {
                Ok(path) if path.is_file() => {
                    let len = path.metadata().map_or(0, |m| m.len());
                    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    Response { status: 200, body: Body::File { path, len, name }, after: None }
                }
                _ => Response::error(404, "no such file"),
            },
        }
    }
}

/// One request answered. `fingerprint`: the SHA-256 of the certificate
/// this connection is made with.
pub fn handle(ctx: &Ctx, req: &Request, body: &mut dyn Read, fingerprint: &[u8]) -> Response {
    let route = (req.method.as_str(), req.path.as_str());
    match route {
        ("GET", "/v1/hello") => {
            let paired = req.token().is_some_and(|t| store::computer_for(&ctx.paths, t).is_some());
            return Response::json(200, ctx.hello(paired));
        }
        ("POST", "/v1/pair") => return ctx.pair_start(req, body, fingerprint),
        ("POST", "/v1/pair/confirm") => return ctx.pair_confirm(req, body, fingerprint),
        _ => {}
    }
    let Some(me) = req.token().and_then(|t| store::computer_for(&ctx.paths, t)) else {
        std::thread::sleep(ctx.penalty);
        return Response::error(401, "this computer is not paired with the handheld");
    };
    match route {
        ("DELETE", "/v1/pair") => {
            let list: Vec<Computer> = store::computers(&ctx.paths).into_iter().filter(|c| c.id != me.id).collect();
            match store::save_computers(&ctx.paths, &list) {
                Ok(()) => {
                    // The last computer gone: the next pairing makes a new certificate.
                    if list.is_empty() {
                        std::fs::remove_file(ctx.paths.cert()).ok();
                        std::fs::remove_file(ctx.paths.key()).ok();
                    }
                    Response::json(200, json!({"forgotten": me.name}))
                }
                Err(_) => Response::error(507, "could not forget it"),
            }
        }
        ("GET", "/v1/status") => {
            let mut v = ctx.hello(true);
            v["computer"] = json!(me.name);
            v["free"] = json!(files::free_space(&ctx.paths.data));
            Response::json(200, v)
        }
        ("GET", "/v1/settings") => shell_reply(ctx.shell.ask("now", "")),
        ("POST", "/v1/settings") => ctx.settings(req, body),
        ("POST", "/v1/music/sign-in") => ctx.sign_in_music(req, body),
        ("DELETE", "/v1/music/sign-in") => shell_reply(ctx.shell.ask("music-sign-out", "")),
        ("POST", "/v1/github") => ctx.github(req, body),
        ("DELETE", "/v1/github") => shell_reply(ctx.shell.ask("forget", "github")),
        ("GET", "/v1/files") | ("GET", "/v1/file") => ctx.files(req),
        ("PUT", "/v1/music") => {
            if req.length == 0 || req.length > SONG_MOST {
                return Response::error(413, "a song comes with its length, 2 GB at most");
            }
            match files::add_song(&ctx.paths, req.query("name").unwrap_or_default(), req.length, body) {
                Ok(rel) => {
                    // The library's index brought up to date (it waits if the shell is away).
                    ctx.shell.ask("music-rescan", "").ok();
                    Response::json(200, json!({"saved": rel}))
                }
                Err(why) => Response::error(422, why),
            }
        }
        ("GET", "/v1/update") => Response::json(200, ctx.update_status()),
        ("POST", "/v1/update/check") => ctx.update_step("check"),
        ("POST", "/v1/update/install") => ctx.update_step("install"),
        ("POST", "/v1/update/apply") => ctx.update_step("apply"),
        ("GET", "/v1/backup") => ctx.backup(),
        ("POST", "/v1/restore") => ctx.restore(body, req.length),
        _ => Response::error(404, "no such request"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{read_request, write_response};

    /// The shell, as the tests have it: what it was asked, and its answers.
    #[derive(Default)]
    struct FakeShell {
        asked: Mutex<Vec<(String, String)>>,
    }

    impl Shell for std::sync::Arc<FakeShell> {
        fn ask(&self, command: &str, body: &str) -> Result<String, String> {
            self.asked.lock().unwrap().push((command.into(), body.into()));
            match command {
                "now" => Ok(r#"{"accent":"rose","music":false}"#.into()),
                "apply" if body.starts_with("cookies=") && !body.contains("SAPISID") => Err("those cookies are not signed in: sign in on music.youtube.com, then export them again".into()),
                _ => Ok(r#"{"received":[["Team","5805"]]}"#.into()),
            }
        }
    }

    const FP: [u8; 32] = [42; 32];

    fn ctx(name: &str) -> (Ctx, std::sync::Arc<FakeShell>) {
        let root = std::env::temp_dir().join(format!("companiond-api-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let paths = Paths { data: root.join("data"), run: root.join("run") };
        std::fs::create_dir_all(&paths.data).unwrap();
        std::fs::create_dir_all(&paths.run).unwrap();
        let shell = std::sync::Arc::new(FakeShell::default());
        let ctx = Ctx { paths, shell: Box::new(shell.clone()), pairing: Mutex::default(), on_device: false, updater: "/nowhere/catalyst-update".into(), penalty: Duration::ZERO };
        (ctx, shell)
    }

    /// A request as it would come over the connection, and its answer's
    /// status and body.
    fn call(ctx: &Ctx, method: &str, path: &str, token: Option<&str>, body: &[u8]) -> (u16, Vec<u8>) {
        let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
        let mut raw = format!("{method} {path} HTTP/1.1\r\nHost: x\r\n{auth}Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        raw.extend_from_slice(body);
        let mut r = &raw[..];
        let req = read_request(&mut r).unwrap().unwrap();
        let response = handle(ctx, &req, &mut r, &FP);
        let status = response.status;
        let mut out = Vec::new();
        write_response(&mut out, response).unwrap();
        let at = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        (status, out[at..].to_vec())
    }

    fn json_of(body: &[u8]) -> Value {
        serde_json::from_slice(body).unwrap()
    }

    fn start_pairing(ctx: &Ctx, code: &str) {
        std::fs::write(ctx.paths.pairing(), format!("{code}\n{}\n", store::now() + 300)).unwrap();
    }

    /// The computer's side of pairing, as the app does it; the token.
    fn pair_as_computer(ctx: &Ctx, code: &str, seen: &[u8]) -> Result<String, (u16, String)> {
        let (computer, first) = pair::Computer::start(code);
        let (status, body) = call(ctx, "POST", "/v1/pair", None, json!({"name": "THOMAS-PC", "message": proto::hex(&first)}).to_string().as_bytes());
        if status != 200 {
            return Err((status, json_of(&body)["error"].as_str().unwrap_or_default().to_owned()));
        }
        let v = json_of(&body);
        let key = computer.finish(&proto::unhex(v["message"].as_str().unwrap()).unwrap()).unwrap();
        let theirs = proto::unhex(v["proof"].as_str().unwrap()).unwrap();
        if !proto::same(&theirs, &pair::confirm(&key, pair::Side::Handheld, seen)) {
            return Err((0, "the handheld's proof did not match".into()));
        }
        let proof = pair::confirm(&key, pair::Side::Computer, seen);
        let (status, body) = call(ctx, "POST", "/v1/pair/confirm", None, json!({"session": v["session"], "proof": proto::hex(&proof)}).to_string().as_bytes());
        let v = json_of(&body);
        if status != 200 {
            return Err((status, v["error"].as_str().unwrap_or_default().to_owned()));
        }
        Ok(v["token"].as_str().unwrap().to_owned())
    }

    #[test]
    fn pairing_with_the_code_on_the_screen_gives_a_token_kept_only_as_its_hash() {
        let (ctx, _) = ctx("pair");
        assert_eq!(pair_as_computer(&ctx, "482913", &FP).unwrap_err().0, 410, "nothing pairs while the handheld is not pairing");
        start_pairing(&ctx, "482913");
        let token = pair_as_computer(&ctx, "482913", &FP).unwrap();
        assert_eq!(token.len(), 64);
        let kept = std::fs::read_to_string(ctx.paths.computers()).unwrap();
        assert!(!kept.contains(&token) && kept.contains(&proto::token_hash(&token)) && kept.contains("THOMAS-PC"));
        assert!(!ctx.paths.pairing().exists(), "the code is spent");
        assert_eq!(std::fs::read_to_string(ctx.paths.pair_result()).unwrap(), "paired\tTHOMAS-PC\n", "for the shell to say so");
        let (status, body) = call(&ctx, "GET", "/v1/hello", Some(&token), b"");
        assert_eq!((status, json_of(&body)["paired"].as_bool()), (200, Some(true)));
        assert_eq!(pair_as_computer(&ctx, "482913", &FP).unwrap_err().0, 410, "nor twice");
    }

    #[test]
    fn a_wrong_code_or_someone_in_the_middle_gets_nothing_and_five_tries_end_it() {
        let (ctx, _) = ctx("wrong");
        start_pairing(&ctx, "482913");
        assert_eq!(pair_as_computer(&ctx, "111111", &FP).unwrap_err().0, 0, "the computer finds out from the handheld's proof");
        assert_eq!(pair_as_computer(&ctx, "482913", &[1u8; 32]).unwrap_err().0, 0, "another certificate in the middle is found out");
        assert!(store::computers(&ctx.paths).is_empty());
        for _ in 0..3 {
            pair_as_computer(&ctx, "000000", &FP).unwrap_err();
        }
        let (status, why) = pair_as_computer(&ctx, "482913", &FP).unwrap_err();
        assert_eq!(status, 429, "five tries, and the right code comes too late: {why}");
        assert!(!ctx.paths.pairing().exists());
        assert_eq!(std::fs::read_to_string(ctx.paths.pair_result()).unwrap(), "refused\n");
    }

    #[test]
    fn a_proof_made_over_another_certificate_is_refused_by_the_handheld() {
        let (ctx, _) = ctx("relay");
        start_pairing(&ctx, "482913");
        let (computer, first) = pair::Computer::start("482913");
        let (_, body) = call(&ctx, "POST", "/v1/pair", None, json!({"name": "x", "message": proto::hex(&first)}).to_string().as_bytes());
        let v = json_of(&body);
        let key = computer.finish(&proto::unhex(v["message"].as_str().unwrap()).unwrap()).unwrap();
        let proof = pair::confirm(&key, pair::Side::Computer, &[9u8; 32]);
        let (status, _) = call(&ctx, "POST", "/v1/pair/confirm", None, json!({"session": v["session"], "proof": proto::hex(&proof)}).to_string().as_bytes());
        assert_eq!(status, 403);
        assert!(store::computers(&ctx.paths).is_empty());
    }

    #[test]
    fn every_request_but_hello_and_pairing_needs_a_paired_token() {
        let (ctx, shell) = ctx("auth");
        for (method, path) in [("GET", "/v1/status"), ("GET", "/v1/settings"), ("POST", "/v1/music/sign-in"), ("GET", "/v1/files?area=music"), ("GET", "/v1/backup"), ("POST", "/v1/restore"), ("DELETE", "/v1/pair")] {
            assert_eq!(call(&ctx, method, path, None, b"").0, 401, "{method} {path}");
            assert_eq!(call(&ctx, method, path, Some("0123456789abcdef"), b"").0, 401, "{method} {path} with a made-up token");
        }
        assert!(shell.asked.lock().unwrap().is_empty(), "the shell was asked nothing");
        assert_eq!(call(&ctx, "GET", "/v1/hello", None, b"").0, 200);
    }

    fn paired(name: &str) -> (Ctx, std::sync::Arc<FakeShell>, String) {
        let (ctx, shell) = ctx(name);
        start_pairing(&ctx, "123456");
        let token = pair_as_computer(&ctx, "123456", &FP).unwrap();
        (ctx, shell, token)
    }

    #[test]
    fn youtube_musics_sign_in_goes_to_the_shells_own_check_and_its_reasons_come_back() {
        let (ctx, shell, token) = paired("music");
        let jar = "# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t1900000000\tSAPISID\tabc\n";
        let (status, _) = call(&ctx, "POST", "/v1/music/sign-in", Some(&token), jar.as_bytes());
        assert_eq!(status, 200);
        let (command, body) = shell.asked.lock().unwrap().last().cloned().unwrap();
        assert_eq!(command, "apply");
        assert_eq!(crate::http::decode(body.strip_prefix("cookies=").unwrap()), jar, "the cookies reach the shell as they were sent");
        let (status, body) = call(&ctx, "POST", "/v1/music/sign-in", Some(&token), b"not cookies");
        assert_eq!(status, 422);
        assert!(json_of(&body)["error"].as_str().unwrap().contains("not signed in"));
        assert_eq!(call(&ctx, "POST", "/v1/music/sign-in", Some(&token), &vec![b'a'; 70 * 1024]).0, 413);
    }

    #[test]
    fn settings_and_the_github_token_go_to_the_shell_as_the_setup_pages_fields() {
        let (ctx, shell, token) = paired("settings");
        let (status, body) = call(&ctx, "POST", "/v1/settings", Some(&token), json!({"team": "5805", "ssid1": "Pit Wi-Fi", "psk1": "p&ss word", "accent": "rose", "calm": true, "choices": "1"}).to_string().as_bytes());
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
        let (_, form) = shell.asked.lock().unwrap().last().cloned().unwrap();
        let fields: Vec<(String, String)> = form.split('&').map(|p| p.split_once('=').unwrap()).map(|(k, v)| (crate::http::decode(k), crate::http::decode(v))).collect();
        assert!(fields.contains(&("psk1".into(), "p&ss word".into())) && fields.contains(&("calm".into(), "on".into())) && fields.contains(&("team".into(), "5805".into())));
        assert_eq!(call(&ctx, "POST", "/v1/settings", Some(&token), br#"{"cookies": "x"}"#).0, 400, "cookies have their own request");
        assert_eq!(call(&ctx, "POST", "/v1/github", Some(&token), br#"{"token": "ghp_abcdefghijklmnopqrstuvwxyz0123456789"}"#).0, 200);
        assert_eq!(shell.asked.lock().unwrap().last().unwrap().1, "github=ghp_abcdefghijklmnopqrstuvwxyz0123456789");
        assert_eq!(call(&ctx, "POST", "/v1/github", Some(&token), br#"{"token": "not a token; rm -rf"}"#).0, 422);
        assert_eq!(call(&ctx, "DELETE", "/v1/github", Some(&token), b"").0, 200);
        assert_eq!(json_of(&call(&ctx, "GET", "/v1/settings", Some(&token), b"").1)["accent"], "rose");
    }

    #[test]
    fn songs_go_into_the_library_and_files_come_back_out() {
        let (ctx, shell, token) = paired("songs");
        let (status, body) = call(&ctx, "PUT", "/v1/music?name=Track%20One.flac", Some(&token), b"fLaC....");
        assert_eq!((status, json_of(&body)["saved"].as_str()), (200, Some("From computer/Track One.flac")));
        assert_eq!(shell.asked.lock().unwrap().last().unwrap().0, "music-rescan", "the library indexes it as its own");
        let (_, body) = call(&ctx, "GET", "/v1/files?area=music&path=From%20computer", Some(&token), b"");
        assert_eq!(json_of(&body)["entries"][0]["name"], "Track One.flac");
        let (status, body) = call(&ctx, "GET", "/v1/file?area=music&path=From%20computer/Track%20One.flac", Some(&token), b"");
        assert_eq!((status, body.as_slice()), (200, &b"fLaC...."[..]));
        assert_eq!(call(&ctx, "GET", "/v1/file?area=music&path=../../companion/computers.json", Some(&token), b"").0, 404);
        assert_eq!(call(&ctx, "PUT", "/v1/music?name=evil.sh", Some(&token), b"#!/bin/sh").0, 422);
    }

    #[test]
    fn a_backup_restores_and_a_computer_can_unpair_itself() {
        let (ctx, _, token) = paired("backup");
        std::fs::create_dir_all(ctx.paths.data.join("notes")).unwrap();
        std::fs::write(ctx.paths.data.join("notes/pit.txt"), "battery 3").unwrap();
        let (status, tar) = call(&ctx, "GET", "/v1/backup", Some(&token), b"");
        assert_eq!(status, 200);
        std::fs::write(ctx.paths.data.join("notes/pit.txt"), "changed").unwrap();
        let (status, body) = call(&ctx, "POST", "/v1/restore", Some(&token), &tar);
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
        assert_eq!(std::fs::read_to_string(ctx.paths.data.join("notes/pit.txt")).unwrap(), "battery 3");
        assert_eq!(store::computers(&ctx.paths).len(), 1, "the backup did not carry the pairing, nor did the restore undo it");
        assert_eq!(call(&ctx, "DELETE", "/v1/pair", Some(&token), b"").0, 200);
        assert_eq!(call(&ctx, "GET", "/v1/status", Some(&token), b"").0, 401, "forgotten");
    }

    #[test]
    fn updates_say_plainly_where_there_is_no_updater() {
        let (ctx, _, token) = paired("update");
        let (status, body) = call(&ctx, "GET", "/v1/update", Some(&token), b"");
        assert_eq!((status, json_of(&body)["available"].as_bool()), (200, Some(false)));
        assert_eq!(call(&ctx, "POST", "/v1/update/install", Some(&token), b"").0, 503);
    }
}
