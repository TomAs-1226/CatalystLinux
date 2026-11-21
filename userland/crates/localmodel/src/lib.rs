//! The on-device model: which models the handheld can run, where they live
//! once downloaded, how llama.cpp's `llama-server` is started for one, and
//! the OpenAI-style chat request it answers. Shared by the shell (the
//! assistant's "On this handheld", Settings › Assistant › On-device model)
//! and `ai` in the Terminal.
//!
//! The models are small enough for a 512 MB handheld with the shell running:
//! SmolLM2 (Hugging Face, Apache-2.0), as GGUF files quantised by bartowski,
//! each pinned here by its size and SHA-256. None is in the image: one is
//! downloaded when asked for, to /data/models. `llama-server` answers on
//! 127.0.0.1 only, and only while something is asking: whoever starts it
//! stops it when done.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

/// One model the handheld can run.
#[derive(Debug, PartialEq)]
pub struct Model {
    /// Its name in the `chosen` file.
    pub id: &'static str,
    /// As people know it.
    pub name: &'static str,
    /// The file it is kept as.
    pub file: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    /// Bytes, exactly: a file of another size is not it.
    pub size: u64,
    /// The context it is run with: room for a question and its answer.
    pub context: u32,
    /// Memory while it answers, measured (MB), and what it is like.
    pub peak_mb: u32,
    pub about: &'static str,
}

/// The models offered, the first the default.
pub const MODELS: [Model; 2] = [
    Model {
        id: "smollm2-135m",
        name: "SmolLM2 135M",
        file: "SmolLM2-135M-Instruct-Q8_0.gguf",
        url: "https://huggingface.co/bartowski/SmolLM2-135M-Instruct-GGUF/resolve/main/SmolLM2-135M-Instruct-Q8_0.gguf",
        sha256: "5a1395716f7913741cc51d98581b9b1228d80987a9f7d3664106742eb06bba83",
        size: 144_811_360,
        context: 2048,
        // llama's buffers as run here: the model mapped (136 MB), an 8-bit
        // cache (24 MB) and a small batch's working space (25 MB).
        peak_mb: 190,
        about: "light; simple answers",
    },
    Model {
        id: "smollm2-360m",
        name: "SmolLM2 360M",
        file: "SmolLM2-360M-Instruct-Q4_K_M.gguf",
        url: "https://huggingface.co/bartowski/SmolLM2-360M-Instruct-GGUF/resolve/main/SmolLM2-360M-Instruct-Q4_K_M.gguf",
        sha256: "2fa3f013dcdd7b99f9b237717fa0b12d75bbb89984cc1274be1471a465bac9c2",
        size: 270_590_880,
        context: 1024,
        // 256 MB mapped, 21 MB of cache, 25 MB working space.
        peak_mb: 305,
        about: "better; slower",
    },
];

/// Where llama-server listens: this handheld only.
pub const HOST: &str = "127.0.0.1";
pub const PORT: u16 = 8088;
/// The program, where the image has it.
pub const SERVER: &str = "/usr/bin/llama-server";

/// The base URL OpenAI-style requests go to (`<base>/chat/completions`).
pub fn base_url() -> String {
    format!("http://{HOST}:{PORT}/v1")
}

/// Where models are kept: `CATALYST_MODELS` when set (tests, the
/// simulator), else /data/models on the handheld, else a folder in the
/// temporary directory.
pub fn dir() -> PathBuf {
    if let Some(d) = std::env::var_os("CATALYST_MODELS") {
        return PathBuf::from(d);
    }
    if cfg!(target_os = "linux") && Path::new("/data").is_dir() {
        return PathBuf::from("/data/models");
    }
    std::env::temp_dir().join("catalyst-models")
}

pub fn by_id(id: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.id == id)
}

/// The model chosen (the `chosen` file in `dir`), else the default.
pub fn chosen(dir: &Path) -> &'static Model {
    std::fs::read_to_string(dir.join("chosen")).ok().and_then(|t| by_id(t.trim())).unwrap_or(&MODELS[0])
}

pub fn set_chosen(dir: &Path, model: &Model) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("chosen"), format!("{}\n", model.id))
}

pub fn path(dir: &Path, model: &Model) -> PathBuf {
    dir.join(model.file)
}

/// The file a download goes to until it is whole and checked.
pub fn partial(dir: &Path, model: &Model) -> PathBuf {
    dir.join(format!("{}.part", model.file))
}

/// Written beside a model once its SHA-256 has been checked.
fn checked_mark(dir: &Path, model: &Model) -> PathBuf {
    dir.join(format!("{}.checked", model.file))
}

/// Whether a model is here, whole and checked.
pub fn present(dir: &Path, model: &Model) -> bool {
    std::fs::metadata(path(dir, model)).is_ok_and(|m| m.len() == model.size) && std::fs::read_to_string(checked_mark(dir, model)).is_ok_and(|s| s.trim() == model.sha256)
}

/// A downloaded file accepted as the model, once its sum has been checked:
/// moved into place and marked.
pub fn accept(dir: &Path, model: &Model, sha256: &str) -> Result<(), String> {
    if sha256 != model.sha256 {
        std::fs::remove_file(partial(dir, model)).ok();
        return Err("the download did not check out: it was thrown away".into());
    }
    std::fs::rename(partial(dir, model), path(dir, model)).map_err(|e| format!("keeping the model: {e}"))?;
    std::fs::write(checked_mark(dir, model), format!("{sha256}\n")).map_err(|e| format!("keeping the model: {e}"))
}

/// A model removed, and anything half-downloaded of it.
pub fn remove(dir: &Path, model: &Model) {
    for p in [path(dir, model), partial(dir, model), checked_mark(dir, model)] {
        std::fs::remove_file(p).ok();
    }
}

/// How llama-server is run for a model: on this handheld's address only,
/// one conversation at a time, three threads (the fourth core stays the
/// shell's), the model mapped rather than copied into memory, and no
/// warm-up pass (the first question is the warm-up). Memory is what limits
/// it here, so: an 8-bit attention cache (half of llama's default 16-bit,
/// which flash attention allows) and small batches, whose working space is
/// a quarter of the default's (measured: 25 MB rather than 99 MB) at no
/// cost to a short question's speed.
pub fn server_args(model_path: &Path, model: &Model) -> Vec<String> {
    [
        "-m",
        &model_path.to_string_lossy(),
        "--host",
        HOST,
        "--port",
        &PORT.to_string(),
        "-c",
        &model.context.to_string(),
        "-t",
        "3",
        "-np",
        "1",
        "-b",
        "256",
        "-ub",
        "128",
        "-fa",
        "on",
        "-ctk",
        "q8_0",
        "-ctv",
        "q8_0",
        "--no-warmup",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

/// Memory free for a program to take now (MB), from the kernel.
pub fn available_mb() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    available_in(&text)
}

fn available_in(meminfo: &str) -> Option<u64> {
    let line = meminfo.lines().find(|l| l.starts_with("MemAvailable:"))?;
    line.split_whitespace().nth(1)?.parse::<u64>().ok().map(|kb| kb / 1024)
}

/// Whether a model can be started with the memory free now: what it takes
/// at its peak and a margin, or why not. (Unknown memory: let it try.)
pub fn room_for(model: &Model, available: Option<u64>) -> Result<(), String> {
    match available {
        Some(free) if free < u64::from(model.peak_mb) + 40 => Err(format!(
            "{} needs about {} MB free and {free} MB is: close the camera or other apps{}",
            model.name,
            model.peak_mb + 40,
            if model.id == MODELS[0].id { "" } else { ", or choose the smaller model" }
        )),
        _ => Ok(()),
    }
}

/// An OpenAI-style chat request: the instructions, then the turns so far as
/// (role, text), "user" and "assistant".
pub fn chat_body(system: &str, turns: &[(&str, &str)], max_tokens: u32) -> Value {
    let mut messages = vec![json!({"role": "system", "content": system})];
    messages.extend(turns.iter().map(|(role, text)| json!({"role": role, "content": text})));
    json!({"messages": messages, "max_tokens": max_tokens, "temperature": 0.3, "stream": false})
}

/// The answer from a chat reply, or why there is none.
pub fn chat_answer(v: &Value) -> Result<String, String> {
    if let Some(message) = v["error"]["message"].as_str() {
        return Err(message.to_owned());
    }
    let text = v["choices"][0]["message"]["content"].as_str().ok_or("the model gave no answer")?;
    Ok(text.trim().to_owned())
}

/// The tokens a reply made per second of generating, when llama-server says.
pub fn speed(v: &Value) -> Option<f64> {
    v["timings"]["predicted_per_second"].as_f64()
}

/// One HTTP request to llama-server, as (status, body). Plain HTTP on the
/// loopback address, so no TLS, no curl: a few lines of HTTP/1.1.
pub fn http(method: &str, path: &str, body: Option<&str>, timeout: Duration) -> Result<(u16, String), String> {
    let mut stream = TcpStream::connect((HOST, PORT)).map_err(|e| format!("the model is not running ({e})"))?;
    stream.set_read_timeout(Some(timeout)).ok();
    stream.set_write_timeout(Some(Duration::from_secs(10))).ok();
    let body = body.unwrap_or("");
    let request = format!("{method} {path} HTTP/1.1\r\nHost: {HOST}:{PORT}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    stream.write_all(request.as_bytes()).map_err(|e| format!("asking the model: {e}"))?;
    read_response(BufReader::new(stream))
}

/// A whole HTTP/1.1 response: its status and body, the body as sent with a
/// length, in chunks, or until the connection closes.
fn read_response(mut reader: impl BufRead) -> Result<(u16, String), String> {
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| format!("reading the answer: {e}"))?;
    let status: u16 = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or("no answer from the model")?;
    let (mut length, mut chunked) = (None, false);
    loop {
        line.clear();
        reader.read_line(&mut line).map_err(|e| format!("reading the answer: {e}"))?;
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            match name.trim().to_ascii_lowercase().as_str() {
                "content-length" => length = value.trim().parse::<usize>().ok(),
                "transfer-encoding" => chunked = value.to_ascii_lowercase().contains("chunked"),
                _ => {}
            }
        }
    }
    let mut body = Vec::new();
    if chunked {
        loop {
            line.clear();
            reader.read_line(&mut line).map_err(|e| format!("reading the answer: {e}"))?;
            let size = usize::from_str_radix(line.trim().split(';').next().unwrap_or("0"), 16).unwrap_or(0);
            if size == 0 {
                break;
            }
            let mut chunk = vec![0u8; size + 2];
            reader.read_exact(&mut chunk).map_err(|e| format!("reading the answer: {e}"))?;
            body.extend_from_slice(&chunk[..size]);
        }
    } else if let Some(n) = length {
        body.resize(n, 0);
        reader.read_exact(&mut body).map_err(|e| format!("reading the answer: {e}"))?;
    } else {
        reader.read_to_end(&mut body).map_err(|e| format!("reading the answer: {e}"))?;
    }
    Ok((status, String::from_utf8_lossy(&body).into_owned()))
}

/// Start llama-server for a model: its own process group (so stopping it
/// stops all of it), a lower priority than the shell (the panel keeps its
/// pace while it thinks), what it prints in /tmp/llama-server.log.
/// `CATALYST_LLAMA_SERVER` names another program (tests).
pub fn spawn_server(dir: &Path, model: &Model) -> Result<std::process::Child, String> {
    if !present(dir, model) {
        return Err(format!("{} is not downloaded: Settings › On-device model", model.name));
    }
    room_for(model, available_mb())?;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        let program = std::env::var("CATALYST_LLAMA_SERVER").unwrap_or_else(|_| SERVER.to_owned());
        let (out, err) = match std::fs::File::create("/tmp/llama-server.log").and_then(|log| Ok((log.try_clone()?, log))) {
            Ok((copy, log)) => (Stdio::from(log), Stdio::from(copy)),
            Err(_) => (Stdio::null(), Stdio::null()),
        };
        let mut command = Command::new(&program);
        command.args(server_args(&path(dir, model), model)).stdin(Stdio::null()).stdout(out).stderr(err).process_group(0);
        let child = command.spawn().map_err(|e| format!("could not start the model ({program}: {e})"))?;
        // SAFETY: a plain system call on the process just started.
        unsafe {
            libc::setpriority(libc::PRIO_PROCESS as _, child.id() as _, 10);
        }
        Ok(child)
    }
    #[cfg(not(unix))]
    {
        Err("the on-device model runs on the handheld".into())
    }
}

/// Wait until a started server answers with its model loaded, or say why not.
pub fn wait_ready(child: &mut std::process::Child, timeout: Duration) -> Result<(), String> {
    let started = std::time::Instant::now();
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("the model stopped while starting ({status}); /tmp/llama-server.log says why"));
        }
        if healthy() {
            return Ok(());
        }
        if started.elapsed() > timeout {
            stop_server(child);
            return Err("the model took too long to start".into());
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// Stop a server started here: asked first, then made to.
pub fn stop_server(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let group = -(child.id() as i32);
        // SAFETY: signals to the process group this program started.
        unsafe {
            libc::kill(group, libc::SIGTERM);
        }
        for _ in 0..20 {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        unsafe {
            libc::kill(group, libc::SIGKILL);
        }
    }
    child.kill().ok();
    child.wait().ok();
}

/// Whether llama-server is up and has its model loaded.
pub fn healthy() -> bool {
    http("GET", "/health", None, Duration::from_secs(2)).is_ok_and(|(status, _)| status == 200)
}

/// Ask the running server a chat question; the answer and its speed.
pub fn ask(body: &Value, timeout: Duration) -> Result<(String, Option<f64>), String> {
    let (status, text) = http("POST", "/v1/chat/completions", Some(&body.to_string()), timeout)?;
    let v: Value = serde_json::from_str(&text).map_err(|_| format!("the model answered {status} with something unreadable"))?;
    if status != 200 {
        return Err(v["error"]["message"].as_str().map_or_else(|| format!("the model answered {status}"), str::to_owned));
    }
    Ok((chat_answer(&v)?, speed(&v)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_models_are_pinned_and_fit() {
        for m in &MODELS {
            assert_eq!(m.sha256.len(), 64, "{}", m.id);
            assert!(m.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
            assert!(m.url.starts_with("https://huggingface.co/") && m.url.ends_with(m.file));
            assert!(m.peak_mb < 400, "{} has to fit beside the shell in 448 MB", m.id);
        }
        assert_eq!(MODELS[0].id, "smollm2-135m", "the light one is the default");
    }

    #[test]
    fn a_model_is_chosen_kept_checked_and_removed() {
        let dir = std::env::temp_dir().join(format!("catalyst-localmodel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(chosen(&dir).id, MODELS[0].id, "the default until one is chosen");
        set_chosen(&dir, &MODELS[1]).unwrap();
        assert_eq!(chosen(&dir).id, MODELS[1].id);
        std::fs::write(dir.join("chosen"), "nonsense").unwrap();
        assert_eq!(chosen(&dir).id, MODELS[0].id, "an unknown one is the default");

        let m = &MODELS[0];
        assert!(!present(&dir, m));
        std::fs::write(partial(&dir, m), b"not it").unwrap();
        assert!(accept(&dir, m, "0000").is_err(), "a wrong sum is refused");
        assert!(!partial(&dir, m).exists(), "and thrown away");
        std::fs::write(partial(&dir, m), b"short").unwrap();
        accept(&dir, m, m.sha256).unwrap();
        assert!(!present(&dir, m), "the wrong size is not the model, whatever the mark says");
        remove(&dir, m);
        assert!(!path(&dir, m).exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_server_listens_on_this_handheld_only() {
        let args = server_args(Path::new("/data/models/x.gguf"), &MODELS[0]);
        let at = |flag: &str| args.iter().position(|a| a == flag).map(|i| args[i + 1].as_str());
        assert_eq!(at("--host"), Some("127.0.0.1"));
        assert_eq!(at("--port"), Some("8088"));
        assert_eq!(at("-c"), Some("2048"));
        assert_eq!(at("-t"), Some("3"));
        assert!(args.contains(&"--no-warmup".to_owned()));
        assert_eq!((at("-ctk"), at("-ctv"), at("-fa"), at("-ub")), (Some("q8_0"), Some("q8_0"), Some("on"), Some("128")), "the memory-saving settings measured");
        assert_eq!(base_url(), "http://127.0.0.1:8088/v1");
    }

    #[test]
    fn a_model_starts_only_with_room_for_it() {
        let meminfo = "MemTotal:  460000 kB\nMemFree:  100000 kB\nMemAvailable:  262144 kB\n";
        assert_eq!(available_in(meminfo), Some(256));
        assert!(room_for(&MODELS[0], Some(256)).is_ok(), "190 + 40 fits in 256");
        let why = room_for(&MODELS[1], Some(256)).unwrap_err();
        assert!(why.contains("345 MB") && why.contains("smaller model"), "{why}");
        assert!(room_for(&MODELS[1], None).is_ok(), "unknown memory: it may try");
    }

    #[test]
    fn a_chat_is_asked_and_answered_in_openai_shape() {
        let body = chat_body("Be brief.", &[("user", "hi"), ("assistant", "hello"), ("user", "and?")], 300);
        assert_eq!(body["messages"][0], json!({"role": "system", "content": "Be brief."}));
        assert_eq!(body["messages"][3], json!({"role": "user", "content": "and?"}));
        assert_eq!((body["max_tokens"].as_u64(), body["stream"].as_bool()), (Some(300), Some(false)));
        let reply = json!({"choices": [{"message": {"role": "assistant", "content": "  Hello!\n"}}], "timings": {"predicted_per_second": 9.5}});
        assert_eq!(chat_answer(&reply).unwrap(), "Hello!");
        assert_eq!(speed(&reply), Some(9.5));
        assert!(chat_answer(&json!({"error": {"message": "context full"}})).is_err());
    }

    #[test]
    fn responses_are_read_by_length_in_chunks_or_to_the_end() {
        let by_length = "HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
        assert_eq!(read_response(by_length.as_bytes()).unwrap(), (200, "hello".into()));
        let chunked = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
        assert_eq!(read_response(chunked.as_bytes()).unwrap(), (200, "abcde".into()));
        let to_end = "HTTP/1.1 503 Service Unavailable\r\n\r\n{\"error\":1}";
        assert_eq!(read_response(to_end.as_bytes()).unwrap(), (503, "{\"error\":1}".into()));
    }
}
