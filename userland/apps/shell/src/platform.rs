//! Where frames go and keys come from: the panel and keyboard on the device,
//! a window on a development PC, or PNG files for scripted screenshots.

use std::time::Duration;

use bezel::Canvas;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Enter,
    /// Esc: leave the current screen.
    Back,
    Backspace,
    Tab,
    PageUp,
    PageDown,
    Char(char),
}

pub trait Platform {
    /// Wait up to `timeout` for keys. `None` means the user closed the shell.
    fn poll(&mut self, timeout: Duration) -> Option<Vec<Key>>;
    /// Show a frame. Returns how much of the panel had to be sent: 0 when the
    /// frame is what is already showing, 1 for the whole panel.
    fn present(&mut self, canvas: &Canvas) -> f32;
    /// A desktop window must be fed every frame to stay responsive; the panel
    /// is only written when the picture changed.
    fn present_always(&self) -> bool {
        false
    }
    /// A line of status for whoever is developing; the device has nowhere to put one.
    fn set_status(&mut self, _text: &str) {}
}

/// The rows that differ from the frame last shown, as first..end, and note
/// this frame as shown. Only those rows are sent to the panel.
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

// ---------------------------------------------------------------- device ---

#[cfg(target_os = "linux")]
pub mod device {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::io::Read;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    const FBIOGET_VSCREENINFO: libc::c_ulong = 0x4600;
    const FBIOGET_FSCREENINFO: libc::c_ulong = 0x4602;

    /// The kernel framebuffer (`/dev/fb0`, the panel's DRM fbdev emulation)
    /// and every evdev keyboard.
    ///
    /// Frames are written to the framebuffer file, not through a memory map.
    /// The panel hangs off SPI and the kernel sends it only what it is told
    /// changed: a write is passed on at once, for exactly the rows written,
    /// while changes made through a map are gathered and sent on a timer,
    /// about twenty times a second, which would cap and unsettle every
    /// animation.
    pub struct Device {
        fb: File,
        width: usize,
        height: usize,
        stride: usize,
        bytes_per_pixel: usize,
        inputs: Vec<File>,
        shift: bool,
        /// The frame the panel is showing, to find what changed.
        shown: Vec<u32>,
        rows: Vec<u8>,
    }

    impl Device {
        pub fn open() -> std::io::Result<Self> {
            let path = std::env::var("CATALYST_FB").unwrap_or_else(|_| "/dev/fb0".into());
            let fb = OpenOptions::new().read(true).write(true).open(path)?;
            // fb_var_screeninfo is 160 bytes, fb_fix_screeninfo 80 on arm64.
            let mut var = [0u32; 40];
            let mut fix = [0u8; 80];
            unsafe {
                if libc::ioctl(fb.as_raw_fd(), FBIOGET_VSCREENINFO as _, var.as_mut_ptr()) != 0
                    || libc::ioctl(fb.as_raw_fd(), FBIOGET_FSCREENINFO as _, fix.as_mut_ptr()) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
            }
            let (width, height, bpp) = (var[0] as usize, var[1] as usize, var[6] as usize);
            // line_length sits after id[16], smem_start (8), smem_len, type, type_aux, visual, 3 × u16 + pad.
            let stride = u32::from_ne_bytes(fix[48..52].try_into().unwrap()) as usize;
            let mut inputs = Vec::new();
            if let Ok(dir) = std::fs::read_dir("/dev/input") {
                for entry in dir.flatten() {
                    if entry.file_name().to_string_lossy().starts_with("event") {
                        if let Ok(f) = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(entry.path()) {
                            inputs.push(f);
                        }
                    }
                }
            }
            Ok(Device { fb, width, height, stride, bytes_per_pixel: bpp / 8, inputs, shift: false, shown: Vec::new(), rows: Vec::new() })
        }

        fn translate(&mut self, code: u16, value: i32) -> Option<Key> {
            const ROWS: [(u16, &str, &str); 4] = [
                (2, "1234567890-=", "!@#$%^&*()_+"),
                (16, "qwertyuiop[]", "QWERTYUIOP{}"),
                (30, "asdfghjkl;'`", "ASDFGHJKL:\"~"),
                (44, "zxcvbnm,./", "ZXCVBNM<>?"),
            ];
            if code == 42 || code == 54 {
                self.shift = value != 0;
                return None;
            }
            if value == 0 {
                return None; // releases
            }
            Some(match code {
                1 => Key::Back,
                14 => Key::Backspace,
                15 => Key::Tab,
                28 | 96 => Key::Enter,
                57 => Key::Char(' '),
                103 => Key::Up,
                108 => Key::Down,
                105 => Key::Left,
                106 => Key::Right,
                104 => Key::PageUp,
                109 => Key::PageDown,
                _ => {
                    let (base, plain, shifted) = ROWS.iter().find(|(base, row, _)| code >= *base && ((code - base) as usize) < row.len())?;
                    let row = if self.shift { shifted } else { plain };
                    Key::Char(row.as_bytes()[(code - base) as usize] as char)
                }
            })
        }
    }

    impl Platform for Device {
        fn poll(&mut self, timeout: Duration) -> Option<Vec<Key>> {
            let mut fds: Vec<libc::pollfd> =
                self.inputs.iter().map(|f| libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLIN, revents: 0 }).collect();
            unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout.as_millis() as i32) };
            let mut keys = Vec::new();
            let mut events = Vec::new();
            for (i, fd) in fds.iter().enumerate() {
                if fd.revents & libc::POLLIN == 0 {
                    continue;
                }
                // struct input_event on 64-bit: 16 bytes of time, type, code, value.
                let mut buf = [0u8; 24 * 16];
                while let Ok(n) = self.inputs[i].read(&mut buf) {
                    if n < 24 {
                        break;
                    }
                    for ev in buf[..n].chunks_exact(24) {
                        let kind = u16::from_ne_bytes([ev[16], ev[17]]);
                        if kind == 1 {
                            events.push((u16::from_ne_bytes([ev[18], ev[19]]), i32::from_ne_bytes(ev[20..24].try_into().unwrap())));
                        }
                    }
                }
            }
            for (code, value) in events {
                keys.extend(self.translate(code, value));
            }
            Some(keys)
        }

        fn present(&mut self, canvas: &Canvas) -> f32 {
            use std::os::unix::fs::FileExt;
            let Some((first, end)) = changed_rows(&mut self.shown, canvas) else { return 0.0 };
            let (w, end) = (self.width.min(canvas.w as usize), end.min(self.height));
            if first >= end {
                return 0.0;
            }
            self.rows.resize((end - first) * self.stride, 0);
            for y in first..end {
                let src = &canvas.px[y * canvas.w as usize..][..w];
                let row = &mut self.rows[(y - first) * self.stride..][..w * self.bytes_per_pixel];
                match self.bytes_per_pixel {
                    2 => {
                        for (o, p) in row.chunks_exact_mut(2).zip(src) {
                            let v = (((p >> 19) & 0x1f) << 11 | ((p >> 10) & 0x3f) << 5 | ((p >> 3) & 0x1f)) as u16;
                            o.copy_from_slice(&v.to_ne_bytes());
                        }
                    }
                    4 => {
                        for (o, p) in row.chunks_exact_mut(4).zip(src) {
                            o.copy_from_slice(&p.to_ne_bytes());
                        }
                    }
                    _ => {}
                }
            }
            // One write for the changed rows: the kernel sends those rows on.
            self.fb.write_all_at(&self.rows, (first * self.stride) as u64).ok();
            (end - first) as f32 / self.height.max(1) as f32
        }
    }
}

// ------------------------------------------------------------ simulator ---

#[cfg(not(target_arch = "aarch64"))]
pub mod sim {
    use super::*;
    use minifb::{Key as K, KeyRepeat, Scale, Window, WindowOptions};

    /// A desktop window showing the panel at 4×.
    pub struct Sim {
        window: Window,
        shown: Vec<u32>,
    }

    impl Sim {
        pub fn open(w: usize, h: usize) -> Self {
            let mut window = Window::new("Catalyst Linux · 320×170 panel", w, h, WindowOptions { scale: Scale::X4, ..Default::default() })
                .expect("open simulator window");
            // The shell starts frames on its own beat, as it does on the device.
            window.set_target_fps(0);
            Sim { window, shown: Vec::new() }
        }
    }

    impl Platform for Sim {
        fn poll(&mut self, timeout: Duration) -> Option<Vec<Key>> {
            if !self.window.is_open() {
                return None;
            }
            // A window has to be looked after often, so never the whole wait.
            std::thread::sleep(timeout.min(Duration::from_millis(8)));
            let shift = self.window.is_key_down(K::LeftShift) || self.window.is_key_down(K::RightShift);
            let keys = self
                .window
                .get_keys_pressed(KeyRepeat::Yes)
                .into_iter()
                .filter_map(|k| {
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
                        K::Space => Key::Char(' '),
                        K::Period => Key::Char('.'),
                        K::Minus => Key::Char('-'),
                        other => {
                            let code = other as usize;
                            let ch = if code <= K::Key9 as usize {
                                (b'0' + (code - K::Key0 as usize) as u8) as char
                            } else if (K::A as usize..=K::Z as usize).contains(&code) {
                                (b'a' + (code - K::A as usize) as u8) as char
                            } else {
                                return None;
                            };
                            Key::Char(if shift { ch.to_ascii_uppercase() } else { ch })
                        }
                    })
                })
                .collect();
            Some(keys)
        }

        fn present_always(&self) -> bool {
            true
        }

        fn set_status(&mut self, text: &str) {
            self.window.set_title(text);
        }

        fn present(&mut self, canvas: &Canvas) -> f32 {
            let part = changed_rows(&mut self.shown, canvas).map_or(0.0, |(first, end)| (end - first) as f32 / canvas.h as f32);
            self.window.update_with_buffer(&canvas.px, canvas.w as usize, canvas.h as usize).ok();
            part
        }
    }

    /// Saves the panel as a PNG, enlarged so single pixels can be inspected.
    pub fn save_png(canvas: &Canvas, path: &std::path::Path, scale: usize) {
        let (w, h) = (canvas.w as usize, canvas.h as usize);
        let mut rgb = vec![0u8; w * scale * h * scale * 3];
        for y in 0..h * scale {
            for x in 0..w * scale {
                let p = canvas.px[(y / scale) * w + x / scale];
                rgb[(y * w * scale + x) * 3..][..3].copy_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
            }
        }
        let file = std::io::BufWriter::new(std::fs::File::create(path).expect("create screenshot"));
        let mut enc = png::Encoder::new(file, (w * scale) as u32, (h * scale) as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&rgb).unwrap();
    }
}
