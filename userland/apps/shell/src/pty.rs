//! A shell to talk to. On the device this is a real one: `/bin/sh` on a
//! pseudo-terminal, so anything that runs in a terminal runs here. On a
//! development PC there is no pseudo-terminal to be had the same way, so a
//! small stand-in shell answers instead, enough to build and show the
//! terminal screen with.

pub struct Session {
    inner: imp::Inner,
}

impl Session {
    /// Start a shell in a terminal of this size.
    pub fn start(cols: usize, rows: usize) -> Session {
        Session { inner: imp::Inner::start(cols, rows, None) }
    }

    /// Run one program (a shell command line) in a terminal of this size,
    /// from the folder it lives in. The session ends when the program does.
    pub fn run(cols: usize, rows: usize, command: &str, dir: &std::path::Path) -> Session {
        Session { inner: imp::Inner::start(cols, rows, Some((command, dir))) }
    }

    /// Whatever the shell has printed since the last read; never waits.
    pub fn read(&mut self) -> Vec<u8> {
        self.inner.read()
    }

    /// Keys typed, or answers to the shell's queries.
    pub fn write(&mut self, bytes: &[u8]) {
        self.inner.write(bytes);
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.inner.resize(cols, rows);
    }

    /// False once the shell has exited.
    pub fn alive(&mut self) -> bool {
        self.inner.alive()
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::ffi::CString;

    pub struct Inner {
        master: libc::c_int,
        child: libc::pid_t,
        exited: bool,
    }

    fn size(cols: usize, rows: usize) -> libc::winsize {
        libc::winsize { ws_row: rows as u16, ws_col: cols as u16, ws_xpixel: 0, ws_ypixel: 0 }
    }

    impl Inner {
        pub fn start(cols: usize, rows: usize, program: Option<(&str, &std::path::Path)>) -> Inner {
            // Everything the child needs is made before the fork: between
            // fork and exec only async-signal-safe calls are allowed.
            let shell = CString::new("/bin/sh").unwrap();
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
            let (argv, start_in) = match program {
                // One program: the shell runs its command line and ends with it.
                Some((command, dir)) => (vec!["sh".to_owned(), "-c".to_owned(), command.to_owned()], dir.to_string_lossy().into_owned()),
                // A leading dash makes it a login shell, which reads /etc/profile.
                None => (vec!["-sh".to_owned()], if std::path::Path::new("/data").is_dir() { "/data".to_owned() } else { home.clone() }),
            };
            let argv: Vec<CString> = argv.into_iter().filter_map(|a| CString::new(a).ok()).collect();
            let env: Vec<CString> = [
                "TERM=xterm".to_owned(),
                format!("HOME={home}"),
                format!("PATH={}", std::env::var("PATH").unwrap_or_else(|_| "/usr/sbin:/usr/bin:/sbin:/bin".into())),
                "PS1=\\w \\$ ".to_owned(),
                "LANG=C.UTF-8".to_owned(),
            ]
            .into_iter()
            .map(|s| CString::new(s).unwrap())
            .collect();
            let start_in = CString::new(start_in).unwrap_or_else(|_| CString::new("/").unwrap());
            let argv_ptrs: Vec<*const libc::c_char> = argv.iter().map(|s| s.as_ptr()).chain(std::iter::once(std::ptr::null())).collect();
            let env_ptrs: Vec<*const libc::c_char> = env.iter().map(|s| s.as_ptr()).chain(std::iter::once(std::ptr::null())).collect();
            let window = size(cols, rows);

            let mut master: libc::c_int = -1;
            let child = unsafe { libc::forkpty(&mut master, std::ptr::null_mut(), std::ptr::null_mut(), &window as *const _ as *mut _) };
            if child == 0 {
                unsafe {
                    libc::chdir(start_in.as_ptr());
                    libc::execve(shell.as_ptr(), argv_ptrs.as_ptr(), env_ptrs.as_ptr());
                    libc::_exit(127);
                }
            }
            if child > 0 {
                unsafe {
                    let flags = libc::fcntl(master, libc::F_GETFL);
                    libc::fcntl(master, libc::F_SETFL, flags | libc::O_NONBLOCK);
                    // Not handed down to whatever the shell starts later (the
                    // Wi-Fi or SSH service, an app): a daemon holding it open
                    // would keep this terminal from ever hanging up.
                    libc::fcntl(master, libc::F_SETFD, libc::FD_CLOEXEC);
                }
            }
            Inner { master, child, exited: child <= 0 }
        }

        pub fn read(&mut self) -> Vec<u8> {
            let mut out = Vec::new();
            let mut buf = [0u8; 4096];
            // Bounded, so a program printing without pause cannot hold the screen up.
            while out.len() < 64 * 1024 && self.master >= 0 {
                let n = unsafe { libc::read(self.master, buf.as_mut_ptr() as *mut _, buf.len()) };
                if n <= 0 {
                    break;
                }
                out.extend_from_slice(&buf[..n as usize]);
            }
            out
        }

        pub fn write(&mut self, bytes: &[u8]) {
            let mut rest = bytes;
            while !rest.is_empty() && self.master >= 0 {
                let n = unsafe { libc::write(self.master, rest.as_ptr() as *const _, rest.len()) };
                if n <= 0 {
                    break;
                }
                rest = &rest[n as usize..];
            }
        }

        pub fn resize(&mut self, cols: usize, rows: usize) {
            if self.master >= 0 {
                let window = size(cols, rows);
                unsafe { libc::ioctl(self.master, libc::TIOCSWINSZ, &window) };
            }
        }

        pub fn alive(&mut self) -> bool {
            if !self.exited {
                let mut status = 0;
                if unsafe { libc::waitpid(self.child, &mut status, libc::WNOHANG) } == self.child {
                    self.exited = true;
                }
            }
            !self.exited
        }
    }

    impl Drop for Inner {
        fn drop(&mut self) {
            unsafe {
                if self.master >= 0 {
                    libc::close(self.master);
                }
                if !self.exited && self.child > 0 {
                    libc::kill(self.child, libc::SIGHUP);
                    // A program takes a moment to go, and must be waited for
                    // once it has, or it stays behind as a zombie for as long
                    // as the shell runs. One that ignores the hang-up is made
                    // to go. Off the drawing thread, which must not wait.
                    let child = self.child;
                    std::thread::Builder::new()
                        .stack_size(64 * 1024)
                        .spawn(move || reap(child))
                        .ok();
                }
            }
        }
    }

    /// Wait for `child` to end, giving it two seconds before making it.
    fn reap(child: libc::pid_t) {
        for _ in 0..40 {
            if unsafe { libc::waitpid(child, std::ptr::null_mut(), libc::WNOHANG) } != 0 {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        unsafe {
            libc::kill(child, libc::SIGKILL);
            libc::waitpid(child, std::ptr::null_mut(), 0);
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    /// A stand-in shell: it echoes, edits a line, and knows a few commands.
    pub struct Inner {
        out: Vec<u8>,
        line: String,
        open: bool,
    }

    const PROMPT: &str = "\x1b[1;34m/data\x1b[0m $ ";

    impl Inner {
        pub fn start(_cols: usize, _rows: usize, program: Option<(&str, &std::path::Path)>) -> Inner {
            let mut out = Vec::new();
            match program {
                Some((command, _)) => {
                    out.extend_from_slice(format!("\x1b[2mstand-in shell: the handheld would run\r\n  {command}\r\nhere (exit finishes it)\x1b[0m\r\n").as_bytes())
                }
                None => out.extend_from_slice(b"\x1b[2mstand-in shell: the handheld runs a real /bin/sh here\x1b[0m\r\n"),
            }
            out.extend_from_slice(PROMPT.as_bytes());
            Inner { out, line: String::new(), open: true }
        }

        pub fn read(&mut self) -> Vec<u8> {
            std::mem::take(&mut self.out)
        }

        pub fn write(&mut self, bytes: &[u8]) {
            for &b in bytes {
                match b {
                    b'\r' | b'\n' => {
                        self.out.extend_from_slice(b"\r\n");
                        let line = std::mem::take(&mut self.line);
                        self.run(line.trim());
                        if self.open {
                            self.out.extend_from_slice(PROMPT.as_bytes());
                        }
                    }
                    0x7f | 0x08 => {
                        if self.line.pop().is_some() {
                            self.out.extend_from_slice(b"\x08 \x08");
                        }
                    }
                    0x03 => {
                        self.line.clear();
                        self.out.extend_from_slice(b"^C\r\n");
                        self.out.extend_from_slice(PROMPT.as_bytes());
                    }
                    0x04 if self.line.is_empty() => self.open = false,
                    0x20..=0x7e => {
                        self.line.push(b as char);
                        self.out.push(b);
                    }
                    _ => {}
                }
            }
        }

        fn run(&mut self, line: &str) {
            let (command, rest) = line.split_once(' ').unwrap_or((line, ""));
            let reply: String = match command {
                "" => String::new(),
                "help" => "help  ls  pwd  uname  echo  colors  clear  exit\r\n".into(),
                "ls" => "\x1b[1;34mcatalyst\x1b[0m  \x1b[1;34mlogs\x1b[0m  \x1b[1;34mwifi\x1b[0m  notes.txt  \x1b[1;32mcheck.sh\x1b[0m\r\n".into(),
                "pwd" => "/data\r\n".into(),
                "uname" => "Linux catalyst 6.18.62-v8 aarch64 GNU/Linux\r\n".into(),
                "echo" => format!("{rest}\r\n"),
                "colors" => {
                    let mut s = String::new();
                    for row in [30, 90] {
                        for c in 0..8 {
                            s += &format!("\x1b[{}m{:>4}\x1b[0m", row + c, row + c);
                        }
                        s += "\r\n";
                    }
                    s
                }
                "clear" => "\x1b[2J\x1b[H".into(),
                "exit" => {
                    self.open = false;
                    String::new()
                }
                other => format!("{other}: not in the stand-in shell (try help)\r\n"),
            };
            self.out.extend_from_slice(reply.as_bytes());
        }

        pub fn resize(&mut self, _cols: usize, _rows: usize) {}

        pub fn alive(&mut self) -> bool {
            self.open
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::term::Term;
    use std::time::{Duration, Instant};

    /// A real shell on a real pseudo-terminal, read through the emulator.
    #[test]
    fn a_real_shell_answers_through_the_emulator() {
        let mut session = Session::start(40, 10);
        let mut term = Term::new(40, 10);
        let pump = |session: &mut Session, term: &mut Term, until: &dyn Fn(&Term) -> bool| {
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(10) {
                let bytes = session.read();
                term.feed(&bytes);
                let replies = term.take_replies();
                if !replies.is_empty() {
                    session.write(&replies);
                }
                if until(term) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            false
        };
        let on_screen = |term: &Term, what: &str| (0..term.rows).any(|y| term.text(y).contains(what));
        assert!(session.alive());
        // The shell says what size it thinks the terminal is, and does arithmetic.
        session.write(b"stty size; echo sum=$((40+2))\r");
        assert!(pump(&mut session, &mut term, &|t| on_screen(t, "sum=42")), "no answer from the shell");
        assert!(on_screen(&term, "10 40"), "the shell was not told the terminal's size");
        // A resize reaches it.
        session.resize(30, 8);
        session.write(b"stty size\r");
        assert!(pump(&mut session, &mut term, &|t| on_screen(t, "8 30")));
        // Leaving ends the session.
        session.write(b"exit\r");
        let start = Instant::now();
        while session.alive() && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!session.alive());
    }
}
