//! MaixPy's raw REPL, as mpremote speaks MicroPython's: Ctrl-C stops what
//! runs, Ctrl-A enters raw mode ("raw REPL; CTRL-B to exit", then `>`), and
//! each piece of code is sent and run with Ctrl-D, answered with what it
//! printed, `04`, its error, `04`, and `>` again.
//!
//! Raw-paste mode (`05 'A' 01`, MicroPython 1.14 on) is asked for first, as
//! mpremote does: "R\x01" and a window size, and the code goes in windows the
//! device opens with `01`. MaixPy's MicroPython is older: it takes the three
//! bytes as code and Ctrl-A, prints its raw banner again, and the code goes
//! the plain way, 256 bytes at a time.
//!
//! Files are written as mpremote writes them: `f=open(...)`, then `w(b'…')`
//! 256 bytes at a time, then `f.close()`; read back as hex to check.

use std::time::Duration;

use super::serial::{Reader, Serial};

const BANNER: &[u8] = b"raw REPL; CTRL-B to exit\r\n";
const BANNER_TAIL: &[u8] = b"w REPL; CTRL-B to exit\r\n>";
/// mpremote's chunk, for code and for a file's bytes.
pub const CHUNK: usize = 256;

pub struct Repl<'a> {
    port: &'a mut dyn Serial,
    rx: Reader,
    /// Raw-paste mode: not asked yet, or whether the device has it.
    paste: Option<bool>,
}

impl<'a> Repl<'a> {
    /// Whatever runs stopped and the raw REPL entered, tried for up to
    /// `within` (a K210 still starting takes Ctrl-C only once MaixPy runs).
    pub fn enter(port: &'a mut dyn Serial, within: Duration) -> Result<Repl<'a>, String> {
        let mut repl = Repl { port, rx: Reader::new(), paste: None };
        let deadline = std::time::Instant::now() + within;
        loop {
            repl.port.write(b"\r\x03");
            std::thread::sleep(Duration::from_millis(100));
            repl.port.write(b"\x03");
            std::thread::sleep(Duration::from_millis(100));
            repl.rx.clear(repl.port);
            repl.port.write(b"\r\x01");
            match repl.rx.until(repl.port, BANNER, Duration::from_millis(900)) {
                Ok(_) => return Ok(repl),
                Err(_heard) => {
                    #[cfg(test)]
                    eprintln!("raw REPL not entered; heard {:?}", String::from_utf8_lossy(&_heard));
                }
            }
            if std::time::Instant::now() >= deadline {
                return Err("no MaixPy answered".into());
            }
        }
    }

    /// Whether the device took raw-paste mode (for tests).
    #[cfg(test)]
    pub fn pasted(&self) -> Option<bool> {
        self.paste
    }

    /// `code` run; what it printed, or its error's last line.
    pub fn exec(&mut self, code: &str, within: Duration) -> Result<String, String> {
        let short = Duration::from_secs(2);
        self.rx.until(self.port, b">", short).map_err(|_| "the camera's REPL did not prompt".to_owned())?;
        let code = code.as_bytes();
        let mut plain = self.paste == Some(false);
        if self.paste.is_none() || self.paste == Some(true) {
            self.port.write(b"\x05A\x01");
            match self.rx.exactly(self.port, 2, short).as_deref() {
                Ok(b"R\x01") => {
                    self.paste = Some(true);
                    self.paste_write(code)?;
                }
                Ok(b"R\x00") => {
                    self.paste = Some(false);
                    plain = true;
                }
                // An older MicroPython: it read Ctrl-A and is in raw mode afresh.
                _ => {
                    self.rx.until(self.port, BANNER_TAIL, short).map_err(|_| "the camera's REPL went astray".to_owned())?;
                    self.paste = Some(false);
                    plain = true;
                }
            }
        }
        if plain {
            for piece in code.chunks(CHUNK) {
                self.port.write(piece);
                std::thread::sleep(Duration::from_millis(10));
            }
            self.port.write(b"\x04");
            if self.rx.exactly(self.port, 2, short).as_deref() != Ok(&b"OK"[..]) {
                return Err("the camera's REPL did not take the code".into());
            }
        }
        let out = self.rx.until(self.port, b"\x04", within).map_err(|_| "the camera did not finish in time".to_owned())?;
        let err = self.rx.until(self.port, b"\x04", short).map_err(|_| "the camera did not finish in time".to_owned())?;
        let err = String::from_utf8_lossy(&err[..err.len() - 1]).into_owned();
        if !err.trim().is_empty() {
            let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("an error").trim().to_owned();
            return Err(format!("the camera said: {last}"));
        }
        Ok(String::from_utf8_lossy(&out[..out.len() - 1]).into_owned())
    }

    fn paste_write(&mut self, code: &[u8]) -> Result<(), String> {
        let short = Duration::from_secs(2);
        let head = self.rx.exactly(self.port, 2, short).map_err(|_| "no raw-paste window".to_owned())?;
        let window = usize::from(u16::from_le_bytes([head[0], head[1]])).max(1);
        let mut room = window;
        let mut at = 0;
        while at < code.len() {
            while room == 0 || self.rx.waiting(self.port) {
                match self.rx.exactly(self.port, 1, short).as_deref() {
                    Ok([0x01]) => room += window,
                    // The device ends it early (an error): acknowledged, and
                    // its error read as any other.
                    Ok([0x04]) => {
                        self.port.write(b"\x04");
                        return Ok(());
                    }
                    _ => return Err("the camera's REPL went astray".into()),
                }
            }
            let n = room.min(code.len() - at);
            self.port.write(&code[at..at + n]);
            room -= n;
            at += n;
        }
        self.port.write(b"\x04");
        self.rx.until(self.port, b"\x04", short).map(|_| ()).map_err(|_| "the camera did not finish the paste".to_owned())
    }

    /// `data` written to `path` on the camera, and read back to check.
    pub fn write_file(&mut self, path: &str, data: &[u8], progress: &mut dyn FnMut(usize)) -> Result<(), String> {
        let t = Duration::from_secs(5);
        self.exec(&format!("f=open('{path}','wb')\nw=f.write"), t)?;
        for (n, piece) in data.chunks(CHUNK).enumerate() {
            self.exec(&format!("w({})", bytes_literal(piece)), t)?;
            progress(n * CHUNK + piece.len());
        }
        self.exec("f.close()", t)?;
        if self.read_file(path)? != data {
            return Err(format!("{path} did not read back the same"));
        }
        Ok(())
    }

    /// A file on the camera, as hex lines read back.
    pub fn read_file(&mut self, path: &str) -> Result<Vec<u8>, String> {
        let code = format!("import ubinascii\nf=open('{path}','rb')\nwhile 1:\n b=f.read(512)\n if not b:\n  break\n print(ubinascii.hexlify(b).decode())\nf.close()");
        let out = self.exec(&code, Duration::from_secs(20))?;
        let hex: String = out.chars().filter(char::is_ascii_hexdigit).collect();
        (0..hex.len() / 2).map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).map_err(|_| "unreadable".to_owned())).collect()
    }

    /// Back to the friendly REPL, and a soft reset: MaixPy starts again, its
    /// boot.py and main.py with it.
    pub fn restart(mut self) {
        self.port.write(b"\x02");
        std::thread::sleep(Duration::from_millis(50));
        self.port.write(b"\x04");
        std::thread::sleep(Duration::from_millis(50));
        self.rx.clear(self.port);
    }
}

/// Bytes as a Python bytes literal (b'…'): printable ASCII as it is, the
/// rest escaped.
pub fn bytes_literal(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() + 3);
    s.push_str("b'");
    for &b in data {
        match b {
            b'\\' => s.push_str("\\\\"),
            b'\'' => s.push_str("\\'"),
            b'\n' => s.push_str("\\n"),
            b'\r' => s.push_str("\\r"),
            b'\t' => s.push_str("\\t"),
            0x20..=0x7E => s.push(char::from(b)),
            _ => s.push_str(&format!("\\x{b:02x}")),
        }
    }
    s.push('\'');
    s
}

/// A bytes literal as `bytes_literal` writes it, read back (the stand-in
/// camera's side).
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn parse_bytes_literal(s: &str) -> Option<Vec<u8>> {
    let body = s.strip_prefix("b'")?.strip_suffix('\'')?.as_bytes();
    let mut out = Vec::with_capacity(body.len());
    let mut i = 0;
    while i < body.len() {
        if body[i] != b'\\' {
            out.push(body[i]);
            i += 1;
            continue;
        }
        let c = *body.get(i + 1)?;
        i += 2;
        out.push(match c {
            b'\\' => b'\\',
            b'\'' => b'\'',
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'x' => {
                let v = u8::from_str_radix(std::str::from_utf8(body.get(i..i + 2)?).ok()?, 16).ok()?;
                i += 2;
                v
            }
            _ => return None,
        });
    }
    Some(out)
}
