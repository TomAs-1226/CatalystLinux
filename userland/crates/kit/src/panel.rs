//! Where an app's frames go and its keys come from: the handheld's panel and
//! keyboard, a window on a computer, or nowhere (tests and screenshots).
//!
//! On the handheld the shell names the panel in `CATALYST_FB` and goes on
//! watching the keyboard for the way out (Ctrl-Q, or Esc held), so an app
//! reads the same keyboards beside it and never needs to say how to leave.

use std::time::Duration;

use bezel::Canvas;

use crate::keys::Key;

pub trait Panel {
    /// Wait up to `timeout` for keys. `None` means the app should end (its
    /// window was closed).
    fn poll(&mut self, timeout: Duration) -> Option<Vec<Key>>;
    /// Show a frame. Only what changed since the last is sent.
    fn present(&mut self, canvas: &Canvas);
    /// A desktop window must be fed every frame to stay responsive.
    fn present_always(&self) -> bool {
        false
    }
}

/// The rows that differ from the frame last shown, as first..end, noting
/// this frame as shown.
pub fn changed_rows(shown: &mut Vec<u32>, canvas: &Canvas) -> Option<(usize, usize)> {
    let (w, h) = (canvas.w as usize, canvas.h as usize);
    if shown.len() != canvas.px.len() {
        *shown = canvas.px.clone();
        return Some((0, h));
    }
    let differs = |y: usize| shown[y * w..(y + 1) * w] != canvas.px[y * w..(y + 1) * w];
    let first = (0..h).find(|y| differs(*y))?;
    let last = (first..h).rev().find(|y| differs(*y)).unwrap_or(first);
    shown[first * w..(last + 1) * w].copy_from_slice(&canvas.px[first * w..(last + 1) * w]);
    Some((first, last + 1))
}

/// Nowhere: frames are kept, keys come from a script. For tests and for
/// rendering an app's screens to pictures.
#[derive(Default)]
pub struct Headless {
    pub keys: std::collections::VecDeque<Key>,
    pub frames: usize,
}

impl Panel for Headless {
    fn poll(&mut self, _timeout: Duration) -> Option<Vec<Key>> {
        Some(self.keys.pop_front().into_iter().collect())
    }
    fn present(&mut self, _canvas: &Canvas) {
        self.frames += 1;
    }
}

/// The handheld's panel, or a window on a computer.
pub fn open() -> Box<dyn Panel> {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("CATALYST_FB").is_some() || std::path::Path::new("/dev/fb0").exists() {
            match device::Device::open() {
                Ok(d) => return Box::new(d),
                Err(why) => eprintln!("catalyst-kit: no panel ({why})"),
            }
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        Box::new(sim::Window::open())
    }
    #[cfg(target_arch = "aarch64")]
    {
        Box::new(Headless::default())
    }
}

// ---------------------------------------------------------------- device ---

#[cfg(target_os = "linux")]
pub mod device {
    use super::*;
    use crate::keys::{translate, Hold, Repeat};
    use std::fs::{File, OpenOptions};
    use std::io::Read;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{FileExt, OpenOptionsExt};

    const FBIOGET_VSCREENINFO: libc::c_ulong = 0x4600;
    const FBIOGET_FSCREENINFO: libc::c_ulong = 0x4602;

    /// The panel's framebuffer and every evdev keyboard. Frames are written
    /// to the framebuffer file (not mapped): the panel hangs off SPI, and a
    /// write is sent on at once for exactly the rows written.
    pub struct Device {
        fb: File,
        width: usize,
        height: usize,
        stride: usize,
        bytes_per_pixel: usize,
        inputs: Vec<(String, File, bool)>,
        shift: bool,
        ctrl: bool,
        esc: Hold,
        esc_down: bool,
        repeat: Repeat<u16>,
        shown: Vec<u32>,
        rows: Vec<u8>,
        /// The tear-free panel mode: the framebuffer is upright (170 × 320)
        /// and the picture is turned a quarter into it, as the shell sends it.
        turned: bool,
    }

    /// The panel the shell named, or the framebuffer the panel's size.
    fn panel_framebuffer() -> String {
        if let Ok(path) = std::env::var("CATALYST_FB") {
            return path;
        }
        let mut found: Vec<String> = std::fs::read_dir("/sys/class/graphics")
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| std::fs::read_to_string(e.path().join("virtual_size")).is_ok_and(|s| matches!(s.trim(), "320,170" | "170,320")))
            .map(|e| format!("/dev/{}", e.file_name().to_string_lossy()))
            .collect();
        found.sort();
        found.into_iter().next().unwrap_or_else(|| "/dev/fb0".into())
    }

    impl Device {
        pub fn open() -> std::io::Result<Self> {
            let path = panel_framebuffer();
            let fb = OpenOptions::new().read(true).write(true).open(&path)?;
            let mut var = [0u32; 40];
            let mut fix = [0u8; 80];
            let (width, height, bpp, stride) = unsafe {
                if libc::ioctl(fb.as_raw_fd(), FBIOGET_VSCREENINFO as _, var.as_mut_ptr()) == 0
                    && libc::ioctl(fb.as_raw_fd(), FBIOGET_FSCREENINFO as _, fix.as_mut_ptr()) == 0
                {
                    (var[0] as usize, var[1] as usize, var[6] as usize, u32::from_ne_bytes(fix[48..52].try_into().unwrap()) as usize)
                } else {
                    // A plain file standing in for the panel (tests, measuring): RGB565, 320 × 170.
                    (320, 170, 16, 640)
                }
            };
            let mut device = Device {
                fb,
                width,
                height,
                stride,
                bytes_per_pixel: bpp / 8,
                inputs: Vec::new(),
                shift: false,
                ctrl: false,
                esc: Hold::default(),
                esc_down: false,
                repeat: Repeat::default(),
                shown: Vec::new(),
                rows: Vec::new(),
                turned: height > width,
            };
            device.find_inputs();
            Ok(device)
        }

        fn find_inputs(&mut self) {
            let present: Vec<String> = std::fs::read_dir("/dev/input")
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with("event"))
                .collect();
            self.inputs.retain(|(name, ..)| present.contains(name));
            for name in present {
                if self.inputs.iter().any(|(open, ..)| *open == name) {
                    continue;
                }
                if let Ok(f) = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(format!("/dev/input/{name}")) {
                    let own = std::fs::read_to_string(format!("/sys/class/input/{name}/device/name")).is_ok_and(|n| n.contains("tca8418"));
                    self.inputs.push((name, f, own));
                }
            }
        }

        fn key(&mut self, code: u16, value: i32, own: bool) -> Option<Key> {
            match code {
                42 | 54 => self.shift = value != 0,
                29 | 97 => self.ctrl = value != 0,
                1 => {
                    self.esc_down = value != 0;
                    return (value == 1).then_some(Key::Back);
                }
                _ if value == 1 => return translate(code, self.shift, self.ctrl, own),
                _ => {}
            }
            None
        }
    }

    impl Panel for Device {
        fn poll(&mut self, timeout: Duration) -> Option<Vec<Key>> {
            let mut fds: Vec<libc::pollfd> = self.inputs.iter().map(|(_, f, _)| libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLIN, revents: 0 }).collect();
            let timeout = self.repeat.next().map_or(timeout, |at| timeout.min(at.saturating_duration_since(std::time::Instant::now())));
            // Esc held is looked at often enough to fire within a beat.
            let timeout = if self.esc_down { timeout.min(Duration::from_millis(50)) } else { timeout };
            if fds.is_empty() {
                std::thread::sleep(timeout);
            } else {
                unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout.as_millis() as i32) };
            }
            let mut events = Vec::new();
            let mut lost = false;
            for (i, fd) in fds.iter().enumerate() {
                if fd.revents & (libc::POLLERR | libc::POLLHUP) != 0 {
                    lost = true;
                }
                if fd.revents & libc::POLLIN == 0 {
                    continue;
                }
                let own = self.inputs[i].2;
                let mut buf = [0u8; 24 * 16];
                while let Ok(n) = self.inputs[i].1.read(&mut buf) {
                    if n < 24 {
                        break;
                    }
                    for ev in buf[..n].chunks_exact(24) {
                        if u16::from_ne_bytes([ev[16], ev[17]]) == 1 {
                            events.push((u16::from_ne_bytes([ev[18], ev[19]]), i32::from_ne_bytes(ev[20..24].try_into().unwrap()), own));
                        }
                    }
                }
            }
            let now = std::time::Instant::now();
            let mut keys = Vec::new();
            for (code, value, own) in events {
                match value {
                    0 => {
                        self.key(code, 0, own);
                        self.repeat.release(code);
                    }
                    1 => {
                        if let Some(key) = self.key(code, 1, own) {
                            self.repeat.press(code, key, now);
                            keys.push(key);
                        }
                    }
                    _ => {}
                }
            }
            keys.extend(self.repeat.due(now));
            if self.esc.update(self.esc_down) {
                keys.push(Key::Leave);
            }
            if lost {
                self.find_inputs();
            }
            Some(keys)
        }

        fn present(&mut self, canvas: &Canvas) {
            let (w, h) = (canvas.w as usize, canvas.h as usize);
            if self.turned {
                // Upright: send the whole picture turned, as the shell's
                // tear-free mode does (column x of the picture is row W−1−x).
                if self.shown == canvas.px {
                    return;
                }
                self.shown.clone_from(&canvas.px);
                self.rows.resize(w * self.stride, 0);
                let mut column = vec![0u32; h];
                for py in 0..w.min(self.height) {
                    for (px, c) in column.iter_mut().enumerate() {
                        *c = canvas.px[px * w + (w - 1 - py)];
                    }
                    let row = &mut self.rows[py * self.stride..][..h * self.bytes_per_pixel];
                    write_row(&column, py, self.bytes_per_pixel, row);
                }
                self.fb.write_all_at(&self.rows, 0).ok();
                return;
            }
            let Some((first, end)) = changed_rows(&mut self.shown, canvas) else { return };
            let (cols, end) = (self.width.min(w), end.min(self.height));
            if first >= end {
                return;
            }
            self.rows.resize((end - first) * self.stride, 0);
            for y in first..end {
                let src = &canvas.px[y * w..][..cols];
                let row = &mut self.rows[(y - first) * self.stride..][..cols * self.bytes_per_pixel];
                write_row(src, y, self.bytes_per_pixel, row);
            }
            self.fb.write_all_at(&self.rows, (first * self.stride) as u64).ok();
        }
    }

    fn write_row(src: &[u32], y: usize, bytes_per_pixel: usize, row: &mut [u8]) {
        match bytes_per_pixel {
            2 => bezel::simd::rgb565_row(src, y, row),
            4 => {
                for (o, p) in row.chunks_exact_mut(4).zip(src) {
                    o.copy_from_slice(&p.to_ne_bytes());
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------- window ---

#[cfg(not(target_arch = "aarch64"))]
pub mod sim {
    use super::*;
    use crate::keys::{Hold, Repeat};
    use minifb::{Key as K, Scale, Window as MiniWindow, WindowOptions};

    /// The panel at 4× in a window, as the shell's simulator shows it:
    /// 16-bit colour, dithered as on the device.
    pub struct Window {
        window: MiniWindow,
        esc: Hold,
        down: Vec<K>,
        repeat: Repeat<K>,
        panel: Vec<u32>,
    }

    impl Window {
        pub fn open() -> Self {
            let title = std::env::var("CATALYST_APP_NAME").unwrap_or_else(|_| "app".into());
            let mut window = MiniWindow::new(&format!("{title} · Catalyst Linux"), 320, 170, WindowOptions { scale: Scale::X4, ..Default::default() }).expect("open a window");
            window.set_target_fps(0);
            Window { window, esc: Hold::default(), down: Vec::new(), repeat: Repeat::default(), panel: Vec::new() }
        }

        fn meaning(k: K, shift: bool, ctrl: bool) -> Option<Key> {
            Some(match k {
                K::Up => Key::Up,
                K::Down => Key::Down,
                K::Left => Key::Left,
                K::Right => Key::Right,
                K::Enter | K::NumPadEnter => Key::Enter,
                K::Escape => Key::Back,
                K::Backspace => Key::Backspace,
                K::Tab => Key::Tab,
                K::PageUp => Key::PageUp,
                K::PageDown => Key::PageDown,
                K::Delete => Key::Delete,
                K::Home => Key::Home,
                K::End => Key::End,
                K::Space => Key::Char(' '),
                K::Period => Key::Char(if shift { '>' } else { '.' }),
                K::Comma => Key::Char(if shift { '<' } else { ',' }),
                K::Minus => Key::Char(if shift { '_' } else { '-' }),
                K::Equal => Key::Char(if shift { '+' } else { '=' }),
                K::Slash => Key::Char(if shift { '?' } else { '/' }),
                K::Semicolon => Key::Char(if shift { ':' } else { ';' }),
                K::Apostrophe => Key::Char(if shift { '"' } else { '\'' }),
                K::LeftBracket => Key::Char(if shift { '{' } else { '[' }),
                K::RightBracket => Key::Char(if shift { '}' } else { ']' }),
                other => {
                    let code = other as usize;
                    const SHIFTED: &[u8; 10] = b")!@#$%^&*(";
                    let ch = if code <= K::Key9 as usize {
                        let d = code - K::Key0 as usize;
                        if shift { SHIFTED[d] as char } else { (b'0' + d as u8) as char }
                    } else if (K::A as usize..=K::Z as usize).contains(&code) {
                        (b'a' + (code - K::A as usize) as u8) as char
                    } else if (K::F1 as usize..=K::F12 as usize).contains(&code) {
                        return Some(Key::F((code - K::F1 as usize + 1) as u8));
                    } else {
                        return None;
                    };
                    if ctrl && ch.is_ascii_alphabetic() {
                        Key::Ctrl(ch)
                    } else if shift && ch.is_ascii_alphabetic() {
                        Key::Char(ch.to_ascii_uppercase())
                    } else {
                        Key::Char(ch)
                    }
                }
            })
        }
    }

    impl Panel for Window {
        fn poll(&mut self, timeout: Duration) -> Option<Vec<Key>> {
            if !self.window.is_open() {
                return None;
            }
            std::thread::sleep(timeout.min(Duration::from_millis(8)));
            self.window.update();
            let now = std::time::Instant::now();
            let shift = self.window.is_key_down(K::LeftShift) || self.window.is_key_down(K::RightShift);
            let ctrl = self.window.is_key_down(K::LeftCtrl) || self.window.is_key_down(K::RightCtrl);
            let leave = self.esc.update(self.window.is_key_down(K::Escape));
            let down = self.window.get_keys();
            for gone in self.down.iter().filter(|k| !down.contains(k)) {
                self.repeat.release(*gone);
            }
            let pressed: Vec<K> = down.iter().copied().filter(|k| !self.down.contains(k)).collect();
            self.down = down;
            let mut keys = Vec::new();
            for k in pressed {
                if let Some(key) = Self::meaning(k, shift, ctrl) {
                    self.repeat.press(k, key, now);
                    keys.push(key);
                }
            }
            keys.extend(self.repeat.due(now));
            if leave {
                keys.push(Key::Leave);
            }
            Some(keys)
        }

        fn present_always(&self) -> bool {
            true
        }

        fn present(&mut self, canvas: &Canvas) {
            let w = canvas.w as usize;
            self.panel.clear();
            self.panel.extend(canvas.px.iter().enumerate().map(|(i, &p)| bezel::canvas::rgb565_to_rgb(bezel::canvas::rgb565_dithered(p, i % w, i / w))));
            self.window.update_with_buffer(&self.panel, w, canvas.h as usize).ok();
        }
    }

    /// The panel as a PNG, each pixel `scale` pixels square, as the shell's
    /// own screenshots are taken (the frame as drawn, before the panel's
    /// 16-bit colour).
    pub fn save_png(canvas: &Canvas, path: &std::path::Path, scale: usize) -> std::io::Result<()> {
        let (w, h) = (canvas.w as usize, canvas.h as usize);
        let mut rgb = vec![0u8; w * scale * h * scale * 3];
        for y in 0..h * scale {
            for x in 0..w * scale {
                let p = canvas.px[(y / scale) * w + x / scale];
                rgb[(y * w * scale + x) * 3..][..3].copy_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
            }
        }
        let file = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut enc = png::Encoder::new(file, (w * scale) as u32, (h * scale) as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().map_err(std::io::Error::other)?.write_image_data(&rgb).map_err(std::io::Error::other)
    }
}
