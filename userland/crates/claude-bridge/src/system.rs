//! Everything the setup does to the computer goes through [`System`]: the
//! programs it runs, the browser it opens, Windows' permission prompt, the
//! Run key, the clock. [`Real`] does it; the tests give a stand-in that
//! plays a made-up computer, so nothing here is ever tried for real by a
//! test.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The programs the setup uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Claude,
    Tailscale,
    Winget,
    PowerShell,
    Powercfg,
    Cmd,
}

/// What a program said and how it ended.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Output {
    /// None: it did not end in time, or could not start.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    /// Its last line that says something, for a failure's words.
    pub fn last_line(&self) -> String {
        let all = format!("{}\n{}", self.stdout, self.stderr);
        all.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").chars().take(160).collect()
    }
}

/// A program left running (a sign-in, Funnel waiting for its approval).
pub trait Job: Send {
    /// What it has said so far, stdout and stderr together.
    fn said(&self) -> String;
    /// Its exit code once it has ended (None inside: no code).
    fn ended(&mut self) -> Option<Option<i32>>;
    /// A line typed to it (a code the browser showed).
    fn type_line(&mut self, line: &str);
    fn stop(&mut self);
}

/// The computer, as the setup reaches it.
pub trait System: Send + Sync {
    /// Where a program is, if it is there.
    fn find(&self, tool: Tool) -> Option<PathBuf>;
    /// Run a program to its end, or until `wait` passes.
    fn run(&self, program: &Path, args: &[&str], wait: Duration) -> Output;
    /// Start one and leave it running; `console` gives it a window of its
    /// own (for a program that needs a terminal to type in).
    fn start(&self, program: &Path, args: &[&str], console: bool) -> Option<Box<dyn Job>>;
    /// Run one with administrator rights: Windows asks the person once.
    /// Err with the reason when they say no or it cannot.
    fn run_elevated(&self, program: &Path, args: &str) -> Result<i32, String>;
    fn open_url(&self, url: &str);
    /// Start with Windows: the user's own Run key.
    fn at_login(&self) -> bool;
    fn set_at_login(&self, on: bool) -> Result<(), String>;
    /// Time, for waiting (a stand-in's runs at once).
    fn now(&self) -> Duration;
    fn sleep(&self, d: Duration);
}

// ----------------------------------------------------------------- real ---

/// The computer itself.
pub struct Real {
    began: Instant,
    /// What Start with Windows runs: this program and its arguments.
    pub at_login_command: String,
}

impl Real {
    pub fn new(at_login_command: String) -> Real {
        Real { began: Instant::now(), at_login_command }
    }
}

fn quiet(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c
}

struct RealJob {
    child: Child,
    said: Arc<Mutex<String>>,
    stdin: Option<std::process::ChildStdin>,
}

impl Job for RealJob {
    fn said(&self) -> String {
        self.said.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn ended(&mut self) -> Option<Option<i32>> {
        match self.child.try_wait() {
            Ok(Some(s)) => Some(s.code()),
            Ok(None) => None,
            Err(_) => Some(None),
        }
    }

    fn type_line(&mut self, line: &str) {
        if let Some(i) = self.stdin.as_mut() {
            let _ = writeln!(i, "{line}");
            let _ = i.flush();
        }
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
    }
}

impl Drop for RealJob {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
        }
    }
}

impl System for Real {
    fn find(&self, tool: Tool) -> Option<PathBuf> {
        let system32 = || PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into())).join("System32");
        match tool {
            Tool::Claude => crate::runner::find_claude(),
            Tool::Tailscale => crate::tailscale::exe(),
            Tool::Winget => {
                let alias = std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Microsoft").join("WindowsApps").join("winget.exe"));
                alias.filter(|p| p.exists()).or_else(|| std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).map(|d| d.join("winget.exe")).find(|p| p.exists()))
            }
            Tool::PowerShell => Some(system32().join("WindowsPowerShell").join("v1.0").join("powershell.exe")).filter(|p| p.is_file()),
            Tool::Powercfg => Some(system32().join("powercfg.exe")).filter(|p| p.is_file()),
            Tool::Cmd => Some(system32().join("cmd.exe")).filter(|p| p.is_file()),
        }
    }

    fn run(&self, program: &Path, args: &[&str], wait: Duration) -> Output {
        let Ok(mut child) = quiet(program).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() else {
            return Output::default();
        };
        let mut out = child.stdout.take();
        let mut err = child.stderr.take();
        let o = std::thread::spawn(move || {
            let mut s = Vec::new();
            if let Some(o) = out.as_mut() {
                let _ = o.read_to_end(&mut s);
            }
            s
        });
        let e = std::thread::spawn(move || {
            let mut s = Vec::new();
            if let Some(e) = err.as_mut() {
                let _ = e.read_to_end(&mut s);
            }
            s
        });
        let until = Instant::now() + wait;
        let code = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s.code(),
                Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(50)),
                _ => {
                    let _ = child.kill();
                    break None;
                }
            }
        };
        let text = |b: Vec<u8>| String::from_utf8_lossy(&b).into_owned();
        Output { code, stdout: o.join().map(text).unwrap_or_default(), stderr: e.join().map(text).unwrap_or_default() }
    }

    fn start(&self, program: &Path, args: &[&str], console: bool) -> Option<Box<dyn Job>> {
        if console {
            let mut c = Command::new(program);
            c.args(args);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
                c.creation_flags(CREATE_NEW_CONSOLE);
            }
            let child = c.spawn().ok()?;
            return Some(Box::new(RealJob { child, said: Arc::new(Mutex::new(String::new())), stdin: None }));
        }
        let mut child = quiet(program).args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().ok()?;
        let said = Arc::new(Mutex::new(String::new()));
        for pipe in [child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>), child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>)].into_iter().flatten() {
            let said = said.clone();
            std::thread::spawn(move || {
                let mut pipe = pipe;
                let mut buf = [0u8; 2048];
                while let Ok(n) = pipe.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let mut s = said.lock().unwrap_or_else(|e| e.into_inner());
                    s.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if s.len() > 64 * 1024 {
                        let cut = s.len() - 32 * 1024;
                        let cut = (cut..s.len()).find(|&i| s.is_char_boundary(i)).unwrap_or(s.len());
                        s.replace_range(..cut, "");
                    }
                }
            });
        }
        let stdin = child.stdin.take();
        Some(Box::new(RealJob { child, said, stdin }))
    }

    fn run_elevated(&self, program: &Path, args: &str) -> Result<i32, String> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
            use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
            use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
            let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
            let (verb, file, params) = (wide("runas"), wide(&program.to_string_lossy()), wide(args));
            // SAFETY: the struct is zeroed then filled with strings that live
            // across the call; the process handle it returns is closed.
            unsafe {
                let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
                info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
                info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
                info.lpVerb = verb.as_ptr();
                info.lpFile = file.as_ptr();
                info.lpParameters = params.as_ptr();
                info.nShow = 0;
                if ShellExecuteExW(&mut info) == 0 {
                    return Err(if GetLastError() == 1223 { "the permission was not given".into() } else { "Windows could not ask for permission".into() });
                }
                if info.hProcess.is_null() {
                    return Err("it did not start".into());
                }
                WaitForSingleObject(info.hProcess, INFINITE);
                let mut code = 1u32;
                GetExitCodeProcess(info.hProcess, &mut code);
                CloseHandle(info.hProcess);
                Ok(code as i32)
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (program, args);
            Err("only on Windows".into())
        }
    }

    fn open_url(&self, url: &str) {
        // Only web pages, never anything a shell would read as more.
        if !(url.starts_with("https://") && url.chars().all(|c| c.is_ascii_graphic())) {
            return;
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::UI::Shell::ShellExecuteW;
            let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
            let (verb, file) = (wide("open"), wide(url));
            // SAFETY: both strings are NUL-terminated and outlive the call.
            unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), 1) };
        }
    }

    fn at_login(&self) -> bool {
        at_login()
    }

    fn set_at_login(&self, on: bool) -> Result<(), String> {
        set_at_login(on, &self.at_login_command)
    }

    fn now(&self) -> Duration {
        self.began.elapsed()
    }

    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

// --------------------------------------------------------- start at login ---

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "Catalyst Claude bridge";

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Whether the bridge starts when this user signs in.
pub fn at_login() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
        let (key, value) = (wide(RUN_KEY), wide(RUN_VALUE));
        let mut size = 0u32;
        // SAFETY: a size query of one value under the user's own key.
        unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) == 0 }
    }
    #[cfg(not(windows))]
    false
}

/// Start the bridge (`command`) when this user signs in, or stop doing so.
pub fn set_at_login(on: bool, command: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Registry::{RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ};
        let command = wide(command);
        let (key_name, value) = (wide(RUN_KEY), wide(RUN_VALUE));
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: the user's own Run key, opened, written and closed.
        unsafe {
            if RegOpenKeyExW(HKEY_CURRENT_USER, key_name.as_ptr(), 0, KEY_SET_VALUE, &mut key) != 0 {
                return Err("Windows would not open its Run settings".into());
            }
            let r = if on { RegSetValueExW(key, value.as_ptr(), 0, REG_SZ, command.as_ptr().cast(), (command.len() * 2) as u32) } else { RegDeleteValueW(key, value.as_ptr()) };
            RegCloseKey(key);
            if r != 0 && on {
                return Err("Windows would not keep the setting".into());
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (on, command);
        Err("only on Windows".into())
    }
}
