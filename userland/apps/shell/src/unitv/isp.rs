//! The K210's ISP, as Kendryte's kflash.py speaks it: its boot ROM (entered
//! with IO16 held low through reset) takes a small loader into SRAM, and
//! that loader (kflash.py's ISP_PROG, isp.bin in the image) writes the SPI
//! flash.
//!
//! Every request is SLIP-framed (`C0` … `C0`, `C0` inside as `DB DC`, `DB`
//! as `DB DD`): the operation (two bytes, little-endian), two zero bytes, a
//! CRC-32 (zlib's) of what follows, then the request's own fields, the
//! address and length first where it has them. Each answer is SLIP-framed
//! too: the operation and a result (`E0` for done).

use std::time::{Duration, Instant};

use super::serial::{wire_time, Serial};
use crate::smartcam::frame::crc32;

// The boot ROM's operations.
pub const ROM_NOP: u8 = 0xC2;
pub const ROM_WRITE: u8 = 0xC3;
pub const ROM_BOOT: u8 = 0xC5;
// The flash loader's.
pub const FLASH_NOP: u8 = 0xD2;
pub const FLASH_WRITE: u8 = 0xD4;
pub const FLASH_BAUD: u8 = 0xD6;
pub const FLASH_INIT: u8 = 0xD7;
/// An answer's result: done.
pub const OK: u8 = 0xE0;
/// Where the loader goes, in SRAM.
pub const SRAM: u32 = 0x8000_0000;
/// The boot ROM takes 1 KiB a request, the loader 64 KiB (kflash.py's
/// ISP_FLASH_DATA_FRAME_SIZE, 16 sectors of 4 KiB), each padded whole.
pub const ROM_CHUNK: usize = 1024;
pub const FLASH_CHUNK: usize = 64 * 1024;
/// Tries at each request before giving up (kflash.py's MAX_RETRY_TIMES).
const TRIES: usize = 10;

/// A packet, SLIP-framed.
pub fn slip(packet: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(packet.len() + 8);
    out.push(0xC0);
    for &b in packet {
        match b {
            0xC0 => out.extend_from_slice(&[0xDB, 0xDC]),
            0xDB => out.extend_from_slice(&[0xDB, 0xDD]),
            _ => out.push(b),
        }
    }
    out.push(0xC0);
    out
}

/// SLIP-framed bytes in, packets out, however they are cut.
#[derive(Default)]
pub struct Unslip {
    packet: Vec<u8>,
    inside: bool,
    escape: bool,
}

impl Unslip {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for &b in bytes {
            match b {
                0xC0 if self.inside && !self.packet.is_empty() => {
                    out.push(std::mem::take(&mut self.packet));
                    self.inside = false;
                }
                // A frame's start (or an empty one: two delimiters).
                0xC0 => {
                    self.inside = true;
                    self.packet.clear();
                }
                _ if !self.inside => {}
                0xDB => self.escape = true,
                0xDC if self.escape => {
                    self.packet.push(0xC0);
                    self.escape = false;
                }
                0xDD if self.escape => {
                    self.packet.push(0xDB);
                    self.escape = false;
                }
                _ => {
                    self.escape = false;
                    self.packet.push(b);
                }
            }
        }
        out
    }
}

/// A request: its operation, two zero bytes, the CRC-32 of `body`, `body`.
pub fn packet(op: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&[op, 0, 0, 0]);
    out.extend_from_slice(&crc32(body).to_le_bytes());
    out.extend_from_slice(body);
    out
}

/// Data to `address` (SRAM through the ROM, flash through the loader).
pub fn write_packet(op: u8, address: u32, data: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(8 + data.len());
    body.extend_from_slice(&address.to_le_bytes());
    body.extend_from_slice(&(data.len() as u32).to_le_bytes());
    body.extend_from_slice(data);
    packet(op, &body)
}

/// Two words: the boot ROM's "run from", the loader's "which flash".
pub fn words_packet(op: u8, a: u32, b: u32) -> Vec<u8> {
    let mut body = a.to_le_bytes().to_vec();
    body.extend_from_slice(&b.to_le_bytes());
    packet(op, &body)
}

/// The loader's baud rate request: 0, 4, the rate.
pub fn baud_packet(baud: u32) -> Vec<u8> {
    let mut body = Vec::with_capacity(12);
    for w in [0, 4, baud] {
        body.extend_from_slice(&w.to_le_bytes());
    }
    packet(FLASH_BAUD, &body)
}

/// kflash.py's greetings, byte for byte: the operation and twelve zero
/// bytes, framed.
pub fn greeting(op: u8) -> Vec<u8> {
    let mut out = vec![0xC0, op];
    out.extend_from_slice(&[0; 12]);
    out.push(0xC0);
    out
}

/// A firmware as the K210's boot ROM finds it at address 0: not encrypted
/// (one zero byte), its length, the firmware, and the SHA-256 of those.
pub fn firmware_image(bin: &[u8]) -> Vec<u8> {
    use sha2::Digest;
    let mut out = Vec::with_capacity(bin.len() + 37);
    out.push(0);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(bin);
    let sum = sha2::Sha256::digest(&out);
    out.extend_from_slice(&sum);
    out
}

/// How a board wires DTR and RTS: kflash.py's "dan" (Sipeed's boards, the
/// M5StickV and UnitV by default: RTS resets, DTR is IO16) or its "kd233"
/// (the other way about; Sipeed's Maix Go and other FT2232 boards).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wiring {
    Dan,
    Kd233,
}

impl Wiring {
    /// (DTR, RTS) for the reset line and the boot pin, `true` asserted.
    fn set(self, port: &mut dyn Serial, reset: bool, boot: bool) {
        match self {
            Wiring::Dan => port.lines(boot, reset),
            Wiring::Kd233 => port.lines(reset, boot),
        }
    }

    /// Into the boot ROM's ISP: reset held, then let go with IO16 low.
    pub fn to_isp(self, port: &mut dyn Serial) {
        self.set(port, false, false);
        std::thread::sleep(Duration::from_millis(100));
        self.set(port, true, false);
        std::thread::sleep(Duration::from_millis(100));
        self.set(port, false, true);
        std::thread::sleep(Duration::from_millis(100));
    }

    /// Reset into whatever is in the flash.
    pub fn to_boot(self, port: &mut dyn Serial) {
        self.set(port, false, false);
        std::thread::sleep(Duration::from_millis(100));
        self.set(port, true, false);
        std::thread::sleep(Duration::from_millis(100));
        self.set(port, false, false);
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A K210 in its ISP, over a serial line.
pub struct Isp<'a> {
    port: &'a mut dyn Serial,
    unslip: Unslip,
    baud: u32,
    pub wiring: Wiring,
}

impl<'a> Isp<'a> {
    /// Into the boot ROM, each wiring tried in turn (`first` first), as
    /// kflash.py's automatic board finding does.
    pub fn enter(port: &'a mut dyn Serial, first: Wiring) -> Result<Isp<'a>, String> {
        port.set_baud(115_200);
        let order = [first, if first == Wiring::Dan { Wiring::Kd233 } else { Wiring::Dan }];
        let mut isp = Isp { port, unslip: Unslip::default(), baud: 115_200, wiring: first };
        for _ in 0..3 {
            for wiring in order {
                wiring.to_isp(isp.port);
                isp.port.discard();
                isp.unslip = Unslip::default();
                isp.port.write(&greeting(ROM_NOP));
                if isp.answer(Duration::from_millis(500)).is_some_and(|(op, _)| op == ROM_NOP) {
                    isp.wiring = wiring;
                    return Ok(isp);
                }
            }
        }
        Err("the camera did not enter its ISP".into())
    }

    /// The next answer: its operation and result.
    fn answer(&mut self, within: Duration) -> Option<(u8, u8)> {
        let deadline = Instant::now() + within;
        let mut buf = [0u8; 256];
        while Instant::now() < deadline {
            let n = self.port.read(&mut buf, Duration::from_millis(20));
            if let Some(p) = self.unslip.push(&buf[..n]).into_iter().find(|p| p.len() >= 2) {
                return Some((p[0], p[1]));
            }
        }
        None
    }

    /// One request, tried until it is answered done.
    fn request(&mut self, framed: &[u8], op: u8, wait: Duration) -> Result<(), String> {
        let within = wait + wire_time(framed.len(), self.baud);
        for _ in 0..TRIES {
            self.port.write(framed);
            match self.answer(within) {
                Some((o, r)) if o == op && (r == OK || (op == ROM_WRITE && r == 0)) => return Ok(()),
                _ => std::thread::sleep(Duration::from_millis(100)),
            }
        }
        Err(format!("the camera's ISP did not take request {op:#04x}"))
    }

    /// The flash loader into SRAM, a kilobyte at a time, and started.
    pub fn load(&mut self, prog: &[u8], progress: &mut dyn FnMut(usize)) -> Result<(), String> {
        for (n, chunk) in prog.chunks(ROM_CHUNK).enumerate() {
            let framed = slip(&write_packet(ROM_WRITE, SRAM + (n * ROM_CHUNK) as u32, chunk));
            self.request(&framed, ROM_WRITE, Duration::from_secs(3))?;
            progress((n * ROM_CHUNK + chunk.len()).min(prog.len()));
        }
        self.port.write(&slip(&words_packet(ROM_BOOT, SRAM, 0)));
        self.port.set_baud(115_200);
        self.baud = 115_200;
        std::thread::sleep(Duration::from_millis(100));
        self.greet()
    }

    /// The loader answering (kflash.py's flash_greeting).
    fn greet(&mut self) -> Result<(), String> {
        for _ in 0..TRIES {
            self.port.write(&greeting(FLASH_NOP));
            if self.answer(Duration::from_secs(3)).is_some_and(|(op, r)| op == FLASH_NOP && r == OK) {
                self.port.discard();
                self.unslip = Unslip::default();
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err("the camera's flash loader did not answer".into())
    }

    /// Faster, if both ends hold it: `false` if the loader stopped answering
    /// (it then has to be loaded again).
    pub fn faster(&mut self, baud: u32) -> bool {
        self.port.write(&slip(&baud_packet(baud)));
        std::thread::sleep(Duration::from_millis(50));
        if !self.port.set_baud(baud) {
            return false;
        }
        self.baud = baud;
        self.greet().is_ok()
    }

    /// The board's own SPI flash chosen (kflash.py's `--flash 1`).
    pub fn init_flash(&mut self) -> Result<(), String> {
        let framed = slip(&words_packet(FLASH_INIT, 1, 0));
        self.request(&framed, FLASH_INIT, Duration::from_secs(3))
    }

    /// `data` into the flash at `address`, 64 KiB at a time (the last
    /// padded with zeros); `progress` hears the bytes written so far.
    pub fn write_flash(&mut self, address: u32, data: &[u8], progress: &mut dyn FnMut(usize)) -> Result<(), String> {
        let mut frame = vec![0u8; FLASH_CHUNK];
        for (n, chunk) in data.chunks(FLASH_CHUNK).enumerate() {
            frame[..chunk.len()].copy_from_slice(chunk);
            frame[chunk.len()..].fill(0);
            let framed = slip(&write_packet(FLASH_WRITE, address + (n * FLASH_CHUNK) as u32, &frame));
            // A sector erased and written takes its time after the bytes.
            self.request(&framed, FLASH_WRITE, Duration::from_secs(4))?;
            progress((n * FLASH_CHUNK + chunk.len()).min(data.len()));
        }
        Ok(())
    }

    /// Done: the camera reset into its flash.
    pub fn finish(self) -> Wiring {
        self.wiring.to_boot(self.port);
        self.port.set_baud(115_200);
        self.wiring
    }
}
