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
    /// A letter with Ctrl held.
    Ctrl(char),
    /// Esc held for a second: leave whatever has the screen, even a program
    /// that takes every other key for itself. (A short Esc is still `Back`.)
    Leave,
}

impl Key {
    /// The two ways out of anything that has the whole keyboard: Ctrl-Q, or
    /// Esc held down.
    pub fn leaves(self) -> bool {
        matches!(self, Key::Ctrl('q') | Key::Leave)
    }
}

/// How long Esc is held before it means [`Key::Leave`].
const HOLD_TO_LEAVE: Duration = Duration::from_secs(1);

/// Watches one key being held down.
#[derive(Default)]
pub struct Hold {
    since: Option<std::time::Instant>,
    fired: bool,
}

impl Hold {
    /// Tell it whether the key is down now; true, once, when it has been
    /// down long enough.
    pub fn update(&mut self, down: bool) -> bool {
        match (down, self.since) {
            (false, _) => {
                self.since = None;
                self.fired = false;
            }
            (true, None) => self.since = Some(std::time::Instant::now()),
            (true, Some(since)) if !self.fired && since.elapsed() >= HOLD_TO_LEAVE => {
                self.fired = true;
                return true;
            }
            _ => {}
        }
        false
    }
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
    /// Something else drew on the panel (an external app): forget what was
    /// last shown, so the next frame is sent whole.
    fn invalidate(&mut self) {}
    /// The panel's framebuffer device, for a program that is handed the screen.
    fn framebuffer(&self) -> Option<String> {
        None
    }
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
        fb_path: String,
        /// Esc held down means leave; whether it is down, and for how long.
        esc: Hold,
        esc_down: bool,
        width: usize,
        height: usize,
        stride: usize,
        bytes_per_pixel: usize,
        /// Every keyboard, by its name under `/dev/input`.
        inputs: Vec<(String, File)>,
        /// Tells when a keyboard is plugged in or pulled out.
        watch: Option<File>,
        shift: bool,
        ctrl: bool,
        /// The frame the panel is showing, to find what changed.
        shown: Vec<u32>,
        rows: Vec<u8>,
        /// The kernel's text console, held in graphics mode while the shell runs.
        _console: Option<File>,
    }

    /// Tell the kernel's text console to stand aside. The boot line puts it
    /// on the panel, and left in text mode it would print kernel messages
    /// over the shell's picture (which is only sent again where it changes,
    /// so they would stay). Its own keyboard handling is turned off too, so
    /// keys meant for the shell cannot switch consoles or restart the
    /// machine. Held for as long as the shell runs; restored when it stops.
    fn quiet_console() -> Option<File> {
        const KDSETMODE: libc::c_ulong = 0x4B3A;
        const KD_GRAPHICS: libc::c_ulong = 0x01;
        const KDSKBMODE: libc::c_ulong = 0x4B45;
        const K_OFF: libc::c_ulong = 0x04;
        let tty = OpenOptions::new().read(true).write(true).custom_flags(libc::O_NOCTTY).open("/dev/tty0").ok()?;
        unsafe {
            libc::ioctl(tty.as_raw_fd(), KDSETMODE as _, KD_GRAPHICS);
            libc::ioctl(tty.as_raw_fd(), KDSKBMODE as _, K_OFF);
        }
        Some(tty)
    }

    impl Drop for Device {
        fn drop(&mut self) {
            const KDSETMODE: libc::c_ulong = 0x4B3A;
            const KD_TEXT: libc::c_ulong = 0x00;
            const KDSKBMODE: libc::c_ulong = 0x4B45;
            const K_UNICODE: libc::c_ulong = 0x03;
            if let Some(tty) = &self._console {
                unsafe {
                    libc::ioctl(tty.as_raw_fd(), KDSKBMODE as _, K_UNICODE);
                    libc::ioctl(tty.as_raw_fd(), KDSETMODE as _, KD_TEXT);
                }
            }
        }
    }

    /// The panel's framebuffer device. Usually `/dev/fb0`, but another
    /// framebuffer (the Pi's HDMI one) can take that name first, so the one
    /// with the panel's size is picked.
    fn panel_framebuffer() -> String {
        if let Ok(path) = std::env::var("CATALYST_FB") {
            return path;
        }
        let mut found: Vec<String> = std::fs::read_dir("/sys/class/graphics")
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| std::fs::read_to_string(e.path().join("virtual_size")).is_ok_and(|size| size.trim() == "320,170"))
            .map(|e| format!("/dev/{}", e.file_name().to_string_lossy()))
            .collect();
        found.sort();
        found.into_iter().next().unwrap_or_else(|| "/dev/fb0".into())
    }

    /// A watch on `/dev/input`, readable whenever a keyboard comes or goes.
    fn watch_inputs() -> Option<File> {
        use std::os::fd::FromRawFd;
        unsafe {
            let fd = libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC);
            if fd < 0 {
                return None;
            }
            let file = File::from_raw_fd(fd);
            (libc::inotify_add_watch(fd, c"/dev/input".as_ptr(), libc::IN_CREATE | libc::IN_DELETE) >= 0).then_some(file)
        }
    }

    impl Device {
        /// Open every keyboard not already open, and let go of unplugged ones.
        fn find_inputs(&mut self) {
            let present: Vec<String> = std::fs::read_dir("/dev/input")
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with("event"))
                .collect();
            self.inputs.retain(|(name, _)| present.contains(name));
            for name in present {
                if self.inputs.iter().any(|(open, _)| *open == name) {
                    continue;
                }
                if let Ok(f) = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(format!("/dev/input/{name}")) {
                    self.inputs.push((name, f));
                }
            }
        }

        pub fn open() -> std::io::Result<Self> {
            let fb_path = panel_framebuffer();
            let fb = OpenOptions::new().read(true).write(true).open(&fb_path)?;
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
            let mut device = Device {
                fb,
                fb_path,
                esc: Hold::default(),
                esc_down: false,
                width,
                height,
                stride,
                bytes_per_pixel: bpp / 8,
                inputs: Vec::new(),
                watch: watch_inputs(),
                shift: false,
                ctrl: false,
                shown: Vec::new(),
                rows: Vec::new(),
                _console: quiet_console(),
            };
            device.find_inputs();
            Ok(device)
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
            if code == 29 || code == 97 {
                self.ctrl = value != 0;
                return None;
            }
            if code == 1 {
                // Esc: once when pressed. Holding it does not repeat; held for
                // a second it becomes Leave (see `poll`).
                self.esc_down = value != 0;
                return (value == 1).then_some(Key::Back);
            }
            if value == 0 {
                return None; // releases
            }
            // Ctrl-Space (or the Opt key, where the keyboard reports one)
            // brings the control centre down: once per press, since a held
            // key repeating would bring it down and put it away in turns.
            if code == 125 || code == 126 || (code == 57 && self.ctrl) {
                return (value == 1).then_some(Key::Ctrl(' '));
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
                    let ch = row.as_bytes()[(code - base) as usize] as char;
                    if self.ctrl && ch.is_ascii_alphabetic() { Key::Ctrl(ch.to_ascii_lowercase()) } else { Key::Char(ch) }
                }
            })
        }
    }

    impl Platform for Device {
        fn poll(&mut self, timeout: Duration) -> Option<Vec<Key>> {
            let mut fds: Vec<libc::pollfd> =
                self.inputs.iter().map(|(_, f)| libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLIN, revents: 0 }).collect();
            if let Some(watch) = &self.watch {
                fds.push(libc::pollfd { fd: watch.as_raw_fd(), events: libc::POLLIN, revents: 0 });
            }
            unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout.as_millis() as i32) };
            let mut keys = Vec::new();
            let mut events = Vec::new();
            let mut changed = false;
            for (i, fd) in fds.iter().enumerate() {
                if i >= self.inputs.len() {
                    // The watch: a keyboard came or went. Empty it and look again below.
                    if fd.revents & libc::POLLIN != 0 {
                        let mut note = [0u8; 1024];
                        while self.watch.as_mut().is_some_and(|w| w.read(&mut note).is_ok_and(|n| n > 0)) {}
                        changed = true;
                    }
                    continue;
                }
                if fd.revents & (libc::POLLERR | libc::POLLHUP) != 0 {
                    changed = true;
                }
                if fd.revents & libc::POLLIN == 0 {
                    continue;
                }
                // struct input_event on 64-bit: 16 bytes of time, type, code, value.
                let mut buf = [0u8; 24 * 16];
                while let Ok(n) = self.inputs[i].1.read(&mut buf) {
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
            if changed {
                self.find_inputs();
            }
            if self.esc.update(self.esc_down) {
                keys.push(Key::Leave);
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
                    // Eight pixels at a time with NEON.
                    2 => bezel::simd::rgb565_row(src, y, row),
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

        fn invalidate(&mut self) {
            self.shown.clear();
        }

        fn framebuffer(&self) -> Option<String> {
            Some(self.fb_path.clone())
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
        esc: Hold,
        /// The frame as the panel would show it.
        panel: Vec<u32>,
    }

    impl Sim {
        pub fn open(w: usize, h: usize) -> Self {
            let mut window = Window::new("Catalyst Linux · 320×170 panel", w, h, WindowOptions { scale: Scale::X4, ..Default::default() })
                .expect("open simulator window");
            // The shell starts frames on its own beat, as it does on the device.
            window.set_target_fps(0);
            #[cfg(windows)]
            show_icon(&window);
            Sim { window, shown: Vec::new(), esc: Hold::default(), panel: Vec::new() }
        }
    }

    /// Catalyst's icon on the window and its taskbar button, from the
    /// program's own resources (`build.rs` puts it there; a test build has none,
    /// and keeps Windows' plain one).
    #[cfg(windows)]
    fn show_icon(window: &Window) {
        use std::ffi::c_void;
        #[link(name = "kernel32")]
        extern "system" {
            fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        }
        #[link(name = "user32")]
        extern "system" {
            fn LoadImageW(instance: *mut c_void, name: *const u16, kind: u32, cx: i32, cy: i32, flags: u32) -> *mut c_void;
            fn GetSystemMetrics(index: i32) -> i32;
            fn SendMessageW(window: *mut c_void, message: u32, wparam: usize, lparam: isize) -> isize;
        }
        const IMAGE_ICON: u32 = 1;
        const WM_SETICON: u32 = 0x0080;
        // The icon's number in the resources, as build.rs names it.
        const ICON_NAME: usize = 1;
        // (ICON_BIG, SM_CXICON) for Alt-Tab and the taskbar, (ICON_SMALL, SM_CXSMICON) for the title bar.
        let sizes = [(1usize, 11), (0usize, 49)];
        // SAFETY: plain Win32 calls on this program's own module and window;
        // a missing icon comes back null and is left alone.
        unsafe {
            let module = GetModuleHandleW(std::ptr::null());
            let hwnd = window.get_window_handle();
            for (which, metric) in sizes {
                let side = GetSystemMetrics(metric);
                let icon = LoadImageW(module, ICON_NAME as *const u16, IMAGE_ICON, side, side, 0);
                if !icon.is_null() {
                    SendMessageW(hwnd, WM_SETICON, which, icon as isize);
                }
            }
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
            let ctrl = self.window.is_key_down(K::LeftCtrl) || self.window.is_key_down(K::RightCtrl);
            let leave = self.esc.update(self.window.is_key_down(K::Escape));
            let mut keys: Vec<Key> = self
                .window
                .get_keys_pressed(KeyRepeat::Yes)
                .into_iter()
                // Esc counts once per press (held, it becomes Leave), and so
                // does Ctrl-Space, which would otherwise open and shut the
                // control centre in turns while held.
                .filter(|k| !(*k == K::Escape || (*k == K::Space && ctrl)) || self.window.is_key_pressed(*k, KeyRepeat::No))
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
                        K::Space if ctrl => Key::Ctrl(' '),
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
                            if ctrl && ch.is_ascii_alphabetic() {
                                Key::Ctrl(ch)
                            } else {
                                Key::Char(if shift { ch.to_ascii_uppercase() } else { ch })
                            }
                        }
                    })
                })
                .collect();
            if leave {
                keys.push(Key::Leave);
            }
            Some(keys)
        }

        fn present_always(&self) -> bool {
            true
        }

        fn invalidate(&mut self) {
            self.shown.clear();
        }

        fn set_status(&mut self, text: &str) {
            self.window.set_title(text);
        }

        fn present(&mut self, canvas: &Canvas) -> f32 {
            let part = changed_rows(&mut self.shown, canvas).map_or(0.0, |(first, end)| (end - first) as f32 / canvas.h as f32);
            // As the panel shows it: 16-bit colour, dithered as on the device.
            let w = canvas.w as usize;
            self.panel.clear();
            self.panel.extend(canvas.px.iter().enumerate().map(|(i, &p)| bezel::canvas::rgb565_to_rgb(bezel::canvas::rgb565_dithered(p, i % w, i / w))));
            self.window.update_with_buffer(&self.panel, w, canvas.h as usize).ok();
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

    /// Frames as an animated PNG, `delay_ms` apart, each pixel `scale`
    /// pixels square: motion that can be watched without a device.
    pub fn save_apng(frames: &[Vec<u32>], w: usize, h: usize, path: &std::path::Path, scale: usize, delay_ms: u16) {
        let file = std::io::BufWriter::new(std::fs::File::create(path).expect("create animation"));
        let mut enc = png::Encoder::new(file, (w * scale) as u32, (h * scale) as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_animated(frames.len() as u32, 0).unwrap();
        enc.set_frame_delay(delay_ms, 1000).unwrap();
        let mut writer = enc.write_header().unwrap();
        let mut rgb = vec![0u8; w * scale * h * scale * 3];
        for frame in frames {
            for y in 0..h * scale {
                for x in 0..w * scale {
                    let p = frame[(y / scale) * w + x / scale];
                    rgb[(y * w * scale + x) * 3..][..3].copy_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
                }
            }
            writer.write_image_data(&rgb).unwrap();
        }
        writer.finish().unwrap();
    }
}
