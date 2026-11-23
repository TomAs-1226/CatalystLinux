//! A stand-in UnitV, for the tests and the simulator: a K210 behind a USB
//! serial bridge as the installer meets one. Its two control lines reset it
//! into its flash or into the boot ROM's ISP (as the board's wiring says);
//! the ROM takes a loader into SRAM and starts it, the loader writes the
//! flash (and checks each request's CRC); MaixPy starts a moment after a
//! reset, runs main.py, and stops it on Ctrl-C; its REPL is MicroPython's
//! as MaixPy's older one has it (no raw-paste mode, unless asked to have
//! it), and runs the few statements the installer sends. Bytes sent at a
//! rate the far end is not at are lost, as on the wire.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::isp::{self, Unslip, Wiring};
use super::repl::parse_bytes_literal;
use super::serial::{wire_time, Serial};
use crate::smartcam::frame;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Firmware {
    /// Nothing that answers (a blank flash, or a program of its own).
    None,
    /// MaixPy; `full` with QR codes, AprilTags and the KPU (not a minimum build).
    MaixPy { full: bool },
}

#[derive(Debug)]
enum Mode {
    Reset,
    Starting { until: Instant },
    /// main.py running.
    Running,
    Friendly,
    Raw { line: Vec<u8> },
    Paste { code: Vec<u8>, window: usize },
    Rom,
    Loader,
    Silent,
}

pub struct Fake {
    pub wiring: Wiring,
    pub firmware: Firmware,
    /// The SPI flash as written.
    pub flash: Vec<u8>,
    /// MaixPy's /flash filesystem.
    pub files: BTreeMap<String, Vec<u8>>,
    /// MicroPython's raw-paste mode (MaixPy's is too old for it).
    pub paste: bool,
    /// From reset to MaixPy taking Ctrl-C.
    pub boot: Duration,
    /// Bytes take their time on the wire, a flash write its own after.
    pub pace: bool,
    /// The bridge's fastest rate.
    pub max_baud: u32,
    /// What happened, for the tests: "isp", "loader", "flash 0x0", "file …".
    pub log: Vec<String>,
    mode: Mode,
    lines: (bool, bool),
    host_baud: u32,
    device_baud: u32,
    out: VecDeque<u8>,
    unslip: Unslip,
    sram: Vec<u8>,
    frames: frame::Decoder,
    open: Option<(String, Vec<u8>)>,
    flash_end: usize,
}

impl Fake {
    pub fn new(firmware: Firmware) -> Fake {
        Fake {
            wiring: Wiring::Dan,
            firmware,
            flash: Vec::new(),
            files: BTreeMap::new(),
            paste: false,
            boot: Duration::from_millis(30),
            pace: false,
            max_baud: 3_000_000,
            log: Vec::new(),
            mode: Mode::Running,
            lines: (false, false),
            host_baud: 115_200,
            device_baud: 115_200,
            out: VecDeque::new(),
            unslip: Unslip::default(),
            sram: Vec::new(),
            frames: frame::Decoder::default(),
            open: None,
            flash_end: usize::MAX,
        }
    }

    /// The camera as running from now: MaixPy started, main.py if any.
    pub fn running(mut self) -> Fake {
        self.start();
        self.mode = if matches!(self.firmware, Firmware::None) { Mode::Silent } else { Mode::Running };
        self
    }

    pub fn shared(self) -> Arc<Mutex<Fake>> {
        Arc::new(Mutex::new(self))
    }

    fn say(&mut self, bytes: &[u8]) {
        self.out.extend(bytes);
    }

    fn start(&mut self) {
        self.mode = self.started();
    }

    /// Reset let go of, or a soft reboot: what it does next.
    fn started(&mut self) -> Mode {
        self.device_baud = 115_200;
        self.frames = frame::Decoder::default();
        match self.firmware {
            Firmware::None => Mode::Silent,
            Firmware::MaixPy { .. } => Mode::Starting { until: Instant::now() + self.boot },
        }
    }

    fn tick(&mut self) {
        if let Mode::Starting { until } = self.mode {
            if Instant::now() >= until {
                self.say(b"\r\n[MAIXPY]Pll0:freq:806000000\r\nMicroPython v0.6.3 on 2024-06-17; Sipeed_M1 with kendryte-k210\r\n");
                self.mode = if self.files.contains_key("/flash/main.py") { Mode::Running } else { Mode::Friendly };
                if matches!(self.mode, Mode::Friendly) {
                    self.say(b">>> ");
                }
            }
        }
    }

    fn set_lines(&mut self, dtr: bool, rts: bool) {
        let ((reset, boot), (was_reset, _)) = match self.wiring {
            Wiring::Dan => ((rts, dtr), (self.lines.1, self.lines.0)),
            Wiring::Kd233 => ((dtr, rts), (self.lines.0, self.lines.1)),
        };
        self.lines = (dtr, rts);
        if reset {
            self.mode = Mode::Reset;
        } else if was_reset {
            self.out.clear();
            if boot {
                self.mode = Mode::Rom;
                self.device_baud = 115_200;
                self.unslip = Unslip::default();
                self.sram.clear();
                self.log.push("isp".into());
            } else {
                self.start();
            }
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        match self.mode {
            Mode::Rom | Mode::Loader => {
                for p in self.unslip.push(bytes) {
                    self.isp(&p);
                }
            }
            _ => {
                for &b in bytes {
                    self.byte(b);
                }
            }
        }
    }

    fn isp(&mut self, p: &[u8]) {
        if p.len() < 8 {
            return;
        }
        let (op, body) = (p[0], &p[8..]);
        let crc_ok = frame::crc32(body) == u32::from_le_bytes([p[4], p[5], p[6], p[7]]);
        let field = |at: usize| body.get(at..at + 4).map_or(0, |w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]));
        let answer = |fake: &mut Fake, ok: bool| fake.say(&isp::slip(&[op, if ok { isp::OK } else { 0xE2 }]));
        let rom = matches!(self.mode, Mode::Rom);
        match (rom, op) {
            (true, isp::ROM_NOP) => answer(self, true),
            (true, isp::ROM_WRITE) if crc_ok => {
                let (at, len) = ((field(0) - isp::SRAM) as usize, field(4) as usize);
                if self.sram.len() < at + len {
                    self.sram.resize(at + len, 0);
                }
                self.sram[at..at + len].copy_from_slice(&body[8..8 + len]);
                answer(self, true);
            }
            (true, isp::ROM_BOOT) if !self.sram.is_empty() => {
                self.mode = Mode::Loader;
                self.unslip = Unslip::default();
                self.log.push("loader".into());
            }
            (false, isp::FLASH_NOP) => answer(self, true),
            (false, isp::FLASH_BAUD) => self.device_baud = field(8),
            (false, isp::FLASH_INIT) => answer(self, crc_ok),
            (false, isp::FLASH_WRITE) if crc_ok => {
                let (at, len) = (field(0) as usize, field(4) as usize);
                if self.flash.len() < at + len {
                    self.flash.resize(at + len, 0xFF);
                }
                self.flash[at..at + len].copy_from_slice(&body[8..8 + len]);
                // Each run of writes noted where it starts.
                if at != self.flash_end {
                    self.log.push(format!("flash {at:#x}"));
                }
                self.flash_end = at + len;
                if self.pace {
                    std::thread::sleep(Duration::from_millis(40));
                }
                self.check_firmware();
                answer(self, true);
            }
            (_, isp::ROM_WRITE | isp::FLASH_WRITE | isp::FLASH_INIT) => answer(self, false),
            _ => {}
        }
    }

    /// A whole firmware image at 0 (its length and SHA-256 right): MaixPy.
    fn check_firmware(&mut self) {
        use sha2::Digest;
        let f = &self.flash;
        if f.len() < 37 || f[0] != 0 {
            return;
        }
        let len = u32::from_le_bytes([f[1], f[2], f[3], f[4]]) as usize;
        if f.len() >= 5 + len + 32 && sha2::Sha256::digest(&f[..5 + len])[..] == f[5 + len..5 + len + 32] {
            self.firmware = Firmware::MaixPy { full: true };
        }
    }

    fn byte(&mut self, b: u8) {
        let mode = std::mem::replace(&mut self.mode, Mode::Silent);
        self.mode = match mode {
            Mode::Running if b == 0x03 => {
                self.say(b"Traceback (most recent call last):\r\n  File \"main.py\", line 180, in <module>\r\nKeyboardInterrupt: \r\nMicroPython v0.6.3; Sipeed_M1 with kendryte-k210\r\n>>> ");
                Mode::Friendly
            }
            Mode::Running => {
                // main.py as installed by this release listens on USB too.
                let listens = self.files.get("/flash/main.py").is_some_and(|m| m.windows(9).any(|w| w == b"repl_uart") || m.windows(13).any(|w| w == b"catalyst_port"));
                for f in self.frames.push(&[b]) {
                    if !listens {
                        continue;
                    }
                    let stamp = self.files.get("/flash/catalyst.json").map(|s| String::from_utf8_lossy(s).into_owned()).unwrap_or_else(|| "null".into());
                    match f.kind {
                        frame::HELLO => {
                            let hello = format!(r#"{{"v":1,"fw":"catalyst-unitv 1","usb":true,"stamp":{stamp}}}"#);
                            self.say(&frame::encode(frame::HELLO_BACK, 1, hello.as_bytes()));
                        }
                        frame::PING => self.say(&frame::encode(frame::PONG, 2, &f.payload)),
                        _ => {}
                    }
                }
                Mode::Running
            }
            Mode::Friendly => match b {
                0x01 => {
                    self.say(b"\r\nraw REPL; CTRL-B to exit\r\n>");
                    Mode::Raw { line: Vec::new() }
                }
                0x03 => {
                    self.say(b"\r\n>>> ");
                    Mode::Friendly
                }
                0x04 => {
                    self.say(b"MPY: soft reboot\r\n");
                    self.started()
                }
                _ => Mode::Friendly,
            },
            Mode::Raw { mut line } => match b {
                0x01 if self.paste && line.as_slice() == [0x05, b'A'] => {
                    self.say(b"R\x01");
                    self.say(&32u16.to_le_bytes());
                    Mode::Paste { code: Vec::new(), window: 32 }
                }
                0x01 => {
                    self.say(b"raw REPL; CTRL-B to exit\r\n>");
                    Mode::Raw { line: Vec::new() }
                }
                0x02 => {
                    self.say(b"\r\n>>> ");
                    Mode::Friendly
                }
                0x03 => Mode::Raw { line: Vec::new() },
                0x04 if line.is_empty() => {
                    self.say(b"OK\r\nMPY: soft reboot\r\n");
                    self.started()
                }
                0x04 => {
                    self.say(b"OK");
                    self.run(&line);
                    Mode::Raw { line: Vec::new() }
                }
                _ => {
                    line.push(b);
                    Mode::Raw { line }
                }
            },
            Mode::Paste { mut code, window } => {
                if b == 0x04 {
                    self.say(b"\x04");
                    self.run(&code);
                    Mode::Raw { line: Vec::new() }
                } else {
                    code.push(b);
                    if code.len() % window == 0 {
                        self.say(b"\x01");
                    }
                    Mode::Paste { code, window }
                }
            }
            other => other,
        };
    }

    /// The statements the installer sends, as MaixPy would run them.
    fn run(&mut self, code: &[u8]) {
        let code = String::from_utf8_lossy(code).into_owned();
        let (out, err) = self.exec(&code);
        self.say(out.replace('\n', "\r\n").as_bytes());
        self.say(b"\x04");
        self.say(err.as_bytes());
        self.say(b"\x04>");
    }

    fn exec(&mut self, code: &str) -> (String, String) {
        let path_in = |code: &str, mode: &str| code.split("open('").nth(1).and_then(|r| r.split_once(&format!("','{mode}')"))).map(|(p, _)| p.to_owned());
        // The install's tidy: an update's leftovers gone; no card in this one.
        if code.contains("CARD|") {
            self.files.retain(|p, _| !p.contains("catalyst.trial") && !p.contains("catalyst.commit") && !p.ends_with(".new") && !p.ends_with(".part"));
            return ("CARD|none\n".into(), String::new());
        }
        if code.contains("CATALYST|") {
            let Firmware::MaixPy { full } = self.firmware else { return (String::new(), "ImportError: no module named 'KPU'\n".into()) };
            let stamp = self.files.get("/flash/catalyst.json").map(|s| String::from_utf8_lossy(s).replace('\n', "")).unwrap_or_else(|| "{}".into());
            return (format!("CATALYST|{}|0.6.3|{stamp}\n", u8::from(full)), String::new());
        }
        if let Some(path) = path_in(code, "wb") {
            self.open = Some((path, Vec::new()));
            return (String::new(), String::new());
        }
        if let Some(lit) = code.strip_prefix("w(").and_then(|r| r.strip_suffix(')')) {
            return match (parse_bytes_literal(lit), self.open.as_mut()) {
                (Some(bytes), Some((_, data))) => {
                    data.extend_from_slice(&bytes);
                    (String::new(), String::new())
                }
                _ => (String::new(), "Traceback (most recent call last):\nNameError: name 'w' isn't defined\n".into()),
            };
        }
        if code == "f.close()" {
            if let Some((path, data)) = self.open.take() {
                self.log.push(format!("file {path}"));
                self.files.insert(path, data);
            }
            return (String::new(), String::new());
        }
        if code.contains("ubinascii.hexlify") {
            let Some(data) = path_in(code, "rb").and_then(|p| self.files.get(&p).cloned()) else {
                return (String::new(), "Traceback (most recent call last):\nOSError: [Errno 2] ENOENT\n".into());
            };
            let hex: Vec<String> = data.chunks(512).map(|c| c.iter().map(|b| format!("{b:02x}")).collect()).collect();
            return (hex.iter().map(|h| format!("{h}\n")).collect(), String::new());
        }
        (String::new(), "Traceback (most recent call last):\n  File \"<stdin>\", line 1\nSyntaxError: invalid syntax\n".into())
    }
}

/// The stand-in as the installer's serial line.
pub struct Port(pub Arc<Mutex<Fake>>);

impl Serial for Port {
    fn set_baud(&mut self, baud: u32) -> bool {
        let mut f = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if baud > f.max_baud {
            return false;
        }
        f.host_baud = baud;
        true
    }

    fn lines(&mut self, dtr: bool, rts: bool) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).set_lines(dtr, rts);
    }

    fn write(&mut self, bytes: &[u8]) -> bool {
        let pace = {
            let mut f = self.0.lock().unwrap_or_else(|e| e.into_inner());
            f.tick();
            if f.host_baud == f.device_baud && !matches!(f.mode, Mode::Reset) {
                f.feed(bytes);
            }
            f.pace.then(|| wire_time(bytes.len(), f.host_baud))
        };
        if let Some(t) = pace {
            std::thread::sleep(t);
        }
        true
    }

    fn read(&mut self, buf: &mut [u8], wait: Duration) -> usize {
        let deadline = Instant::now() + wait;
        loop {
            {
                let mut f = self.0.lock().unwrap_or_else(|e| e.into_inner());
                f.tick();
                if f.host_baud != f.device_baud {
                    f.out.clear();
                }
                if !f.out.is_empty() {
                    let n = buf.len().min(f.out.len());
                    for (i, b) in f.out.drain(..n).enumerate() {
                        buf[i] = b;
                    }
                    return n;
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return 0;
            }
            std::thread::sleep((deadline - now).min(Duration::from_millis(2)));
        }
    }

    fn discard(&mut self) {
        let mut f = self.0.lock().unwrap_or_else(|e| e.into_inner());
        f.tick();
        f.out.clear();
    }
}
