//! One turn of Claude Code, headless: `claude -p` in the session's folder,
//! the message on its standard input, stream-json read from its output.
//!
//! The command line, every flag as `claude --help` (2.1.325) and the CLI
//! reference list it:
//!
//! ```text
//! claude -p --output-format stream-json --verbose --include-partial-messages
//!        --permission-mode default|acceptEdits
//!        --mcp-config <file> --permission-prompt-tool mcp__catalyst-handheld__approve
//!        --resume <session id>        (a session carried on)
//!        --session-id <new uuid>      (a new one, so its id is known at once)
//! ```
//!
//! - Nothing secret is on it: the MCP config is a file in the bridge's own
//!   folder (the permission tool's one-turn ticket inside it), deleted when
//!   the turn ends; the message goes on standard input.
//! - The permission mode is always given, and only ever one of those two,
//!   so a `defaultMode` in a settings file never applies and the handheld
//!   can never ask for `bypassPermissions` or `auto`.
//! - On Windows the turn runs in a job object that ends everything in it
//!   when it closes, so stopping a turn stops the commands it started too,
//!   and with no console window.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

/// The permission prompt tool's full name: the bridge's MCP server
/// (`catalyst-handheld`) and its tool (`approve`).
pub const SERVER: &str = "catalyst-handheld";
pub const TOOL: &str = "mcp__catalyst-handheld__approve";

/// How a session asks: for everything, or with edits in its folder accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Ask,
    Edits,
}

impl Mode {
    /// The value for `--permission-mode` (`default` is Manual mode; the CLI
    /// takes `manual` as its alias).
    pub fn flag(self) -> &'static str {
        match self {
            Mode::Ask => "default",
            Mode::Edits => "acceptEdits",
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Mode::Ask => "ask",
            Mode::Edits => "edits",
        }
    }

    /// From the handheld's word: those two and nothing else.
    pub fn from_word(w: &str) -> Option<Mode> {
        match w {
            "ask" => Some(Mode::Ask),
            "edits" => Some(Mode::Edits),
            _ => None,
        }
    }
}

/// What one turn runs.
pub struct Launch {
    pub claude: PathBuf,
    pub cwd: PathBuf,
    pub session: String,
    /// Carry the session on (`--resume`), or begin it (`--session-id`).
    pub resume: bool,
    pub mode: Mode,
    pub mcp_config: PathBuf,
}

/// The arguments for `claude`.
pub fn args(l: &Launch) -> Vec<OsString> {
    let mut a: Vec<OsString> = ["-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages", "--permission-mode", l.mode.flag(), "--mcp-config"].iter().map(OsString::from).collect();
    a.push(l.mcp_config.clone().into_os_string());
    a.push("--permission-prompt-tool".into());
    a.push(TOOL.into());
    a.push(if l.resume { "--resume" } else { "--session-id" }.into());
    a.push(l.session.clone().into());
    a
}

/// The MCP config a turn's claude is given: the bridge's permission tool,
/// run as `command args`, told the port and the turn's ticket.
pub fn mcp_config(command: &Path, command_args: &[String], port: u16, ticket: &str) -> serde_json::Value {
    serde_json::json!({"mcpServers": {SERVER: {
        "type": "stdio",
        "command": command.to_string_lossy(),
        "args": command_args,
        "env": {"CATALYST_APPROVER_PORT": port.to_string(), "CATALYST_APPROVER_TICKET": ticket},
    }}})
}

/// A turn running.
pub struct Running {
    child: Arc<Mutex<Child>>,
    #[cfg(windows)]
    job: Job,
}

impl Running {
    /// Stop it, and everything it started.
    pub fn stop(&self) {
        #[cfg(windows)]
        self.job.terminate();
        if let Ok(mut c) = self.child.lock() {
            let _ = c.kill();
        }
    }
}

/// Where claude is: `CATALYST_CLAUDE`, Claude Code's own installer's
/// place, the PATH, then npm's (whose `claude.cmd` only runs the
/// `claude.exe` in its package, which is run here directly).
pub fn find_claude() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CATALYST_CLAUDE").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default();
    let exe = if cfg!(windows) { "claude.exe" } else { "claude" };
    let npm_exe = |dir: &Path| dir.join("node_modules").join("@anthropic-ai").join("claude-code").join("bin").join(exe);
    let mut places = vec![home.join(".local").join("bin").join(exe), home.join(".claude").join("local").join(exe)];
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        places.push(dir.join(exe));
        places.push(npm_exe(&dir));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        places.push(npm_exe(&PathBuf::from(appdata).join("npm")));
    }
    places.into_iter().find(|p| p.is_file())
}

/// Whether Claude Code is signed in on this computer: `claude auth status
/// --json`, its `loggedIn` alone read (never the account it names). None
/// when it cannot tell.
pub fn signed_in(claude: &Path) -> Option<bool> {
    let mut cmd = Command::new(claude);
    cmd.args(["auth", "status", "--json"]).stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok()?;
    serde_json::from_slice::<serde_json::Value>(&out.stdout).ok()?["loggedIn"].as_bool()
}

/// Start a turn: `prompt` sent on standard input, each line of output to
/// `on_line` (on a thread of its own), and `on_exit` once it has ended,
/// with the last of what it said on standard error (never logged).
pub fn start(l: &Launch, prompt: &str, on_line: impl FnMut(&str) + Send + 'static, on_exit: impl FnOnce(Option<i32>, String) + Send + 'static) -> std::io::Result<Running> {
    let mut cmd = Command::new(&l.claude);
    cmd.args(args(l)).current_dir(&l.cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn()?;
    #[cfg(windows)]
    let job = Job::new();
    #[cfg(windows)]
    job.assign(&child);
    let mut stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let prompt = prompt.to_owned();
    std::thread::spawn(move || {
        if let Some(s) = stdin.as_mut() {
            let _ = s.write_all(prompt.as_bytes());
        }
        drop(stdin);
    });
    let said = Arc::new(Mutex::new(String::new()));
    let said_to = said.clone();
    let err_thread = std::thread::spawn(move || {
        let Some(mut e) = stderr else { return };
        let mut buf = [0u8; 4096];
        while let Ok(n) = e.read(&mut buf) {
            if n == 0 {
                break;
            }
            let mut s = said_to.lock().unwrap_or_else(|e| e.into_inner());
            s.push_str(&String::from_utf8_lossy(&buf[..n]));
            // Only the end is kept.
            if s.len() > 4096 {
                let cut = s.len() - 2048;
                let cut = (cut..s.len()).find(|&i| s.is_char_boundary(i)).unwrap_or(s.len());
                s.replace_range(..cut, "");
            }
        }
    });
    let child = Arc::new(Mutex::new(child));
    let waiter = child.clone();
    let mut on_line = on_line;
    std::thread::spawn(move || {
        if let Some(out) = stdout {
            let mut reader = BufReader::new(out);
            let mut line = Vec::new();
            loop {
                line.clear();
                match reader.read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => on_line(&String::from_utf8_lossy(&line)),
                }
            }
        }
        let _ = err_thread.join();
        let code = loop {
            let status = waiter.lock().unwrap_or_else(|e| e.into_inner()).try_wait();
            match status {
                Ok(Some(s)) => break s.code(),
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(20)),
                Err(_) => break None,
            }
        };
        let said = std::mem::take(&mut *said.lock().unwrap_or_else(|e| e.into_inner()));
        on_exit(code, said);
    });
    Ok(Running {
        child,
        #[cfg(windows)]
        job,
    })
}

/// A job object that ends its processes when it is closed or terminated.
#[cfg(windows)]
struct Job(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
unsafe impl Send for Job {}
#[cfg(windows)]
unsafe impl Sync for Job {}

#[cfg(windows)]
impl Job {
    fn new() -> Job {
        use windows_sys::Win32::System::JobObjects::*;
        // SAFETY: a new unnamed job; its limit set from a zeroed struct.
        unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if !h.is_null() {
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(h, JobObjectExtendedLimitInformation, (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(), std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32);
            }
            Job(h)
        }
    }

    fn assign(&self, child: &Child) {
        use std::os::windows::io::AsRawHandle;
        if self.0.is_null() {
            return;
        }
        // SAFETY: the child's process handle, valid while `child` lives.
        unsafe { windows_sys::Win32::System::JobObjects::AssignProcessToJobObject(self.0, child.as_raw_handle() as _) };
    }

    fn terminate(&self) {
        if !self.0.is_null() {
            // SAFETY: our own job handle.
            unsafe { windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0, 1) };
        }
    }
}

#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: closed once; with kill-on-close, what is left in it ends.
            unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_is_the_documented_one_and_holds_no_secret() {
        let l = Launch { claude: "claude".into(), cwd: ".".into(), session: "3f0c2a9e-1b2c-4d5e-8f90-a1b2c3d4e5f6".into(), resume: true, mode: Mode::Edits, mcp_config: "C:\\state\\run\\t1.json".into() };
        let a: Vec<String> = args(&l).iter().map(|s| s.to_string_lossy().into_owned()).collect();
        assert_eq!(
            a,
            ["-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages", "--permission-mode", "acceptEdits", "--mcp-config", "C:\\state\\run\\t1.json", "--permission-prompt-tool", "mcp__catalyst-handheld__approve", "--resume", "3f0c2a9e-1b2c-4d5e-8f90-a1b2c3d4e5f6"]
        );
        let new = Launch { resume: false, mode: Mode::Ask, ..l };
        let a: Vec<String> = args(&new).iter().map(|s| s.to_string_lossy().into_owned()).collect();
        assert!(a.windows(2).any(|w| w == ["--permission-mode", "default"]) && a.windows(2).any(|w| w[0] == "--session-id"));
        assert!(!a.iter().any(|x| x.contains("bypass") || x.contains("dangerously") || x == "auto"));
        assert_eq!(Mode::from_word("bypassPermissions"), None);
        assert_eq!(Mode::from_word("auto"), None);
    }
}
