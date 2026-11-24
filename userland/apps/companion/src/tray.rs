//! The Claude bridge's place in the tray: Catalyst's bolt, a tooltip that
//! says how it stands, and a menu (Claude on the handheld…, Quit). Its
//! window is message-only, so it has no taskbar button; a second
//! `--claude-bridge` finds it by its class and asks it to show the window
//! rather than starting again.

#![cfg(windows)]

use std::sync::mpsc::Sender;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// What the tray asks of the bridge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ask {
    Show,
    Quit,
}

const CLASS: &str = "CatalystClaudeBridge";
const CALLBACK: u32 = WM_APP + 1;
/// Posted by a second instance: show the window.
pub const SHOW: u32 = WM_APP + 2;
/// Posted by the bridge: the tooltip changed.
const TIP: u32 = WM_APP + 3;
/// Posted by the bridge: a balloon to show.
const NOTE: u32 = WM_APP + 4;
const MENU_SHOW: usize = 1;
const MENU_QUIT: usize = 2;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

static SENDER: std::sync::Mutex<Option<Sender<Ask>>> = std::sync::Mutex::new(None);
static TIP_TEXT: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
static NOTE_TEXT: std::sync::Mutex<(String, String)> = std::sync::Mutex::new((String::new(), String::new()));

fn copy_into(to: &mut [u16], s: &str) {
    let w: Vec<u16> = s.encode_utf16().take(to.len() - 1).collect();
    to.fill(0);
    to[..w.len()].copy_from_slice(&w);
}

fn send(a: Ask) {
    if let Some(s) = SENDER.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = s.send(a);
    }
}

/// The running bridge's tray window, if there is one: asked to show itself.
pub fn show_running() -> bool {
    let class = wide(CLASS);
    // SAFETY: a lookup among message-only windows by our class name.
    unsafe {
        let hwnd = FindWindowExW(HWND_MESSAGE, std::ptr::null_mut(), class.as_ptr(), std::ptr::null());
        if hwnd.is_null() {
            return false;
        }
        PostMessageW(hwnd, SHOW, 0, 0);
    }
    true
}

/// The bolt as an icon: the app's own 32 px picture.
fn icon() -> HICON {
    let png = include_bytes!("../../../assets/app-icon/icon-32.png");
    let decoder = png::Decoder::new(std::io::Cursor::new(&png[..]));
    let Ok(mut reader) = decoder.read_info() else { return std::ptr::null_mut() };
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap_or(32 * 32 * 4)];
    let Ok(info) = reader.next_frame(&mut buf) else { return std::ptr::null_mut() };
    let (w, h) = (info.width as i32, info.height as i32);
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf[..(w * h * 4) as usize].to_vec(),
        png::ColorType::Rgb => buf[..(w * h * 3) as usize].chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        _ => return std::ptr::null_mut(),
    };
    let bgra: Vec<u8> = rgba.chunks(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
    let mask = vec![0u8; ((w + 15) / 16 * 2 * h) as usize];
    // SAFETY: bitmaps made from buffers that outlive the calls; deleted
    // once the icon has copied them.
    unsafe {
        let color = CreateBitmap(w, h, 1, 32, bgra.as_ptr().cast());
        let mono = CreateBitmap(w, h, 1, 1, mask.as_ptr().cast());
        let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mono, hbmColor: color };
        let icon = CreateIconIndirect(&info);
        DeleteObject(color as _);
        DeleteObject(mono as _);
        icon
    }
}

fn data(hwnd: HWND) -> NOTIFYICONDATAW {
    // SAFETY: a plain struct, zeroed then filled.
    let mut d: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    d.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    d.hWnd = hwnd;
    d.uID = 1;
    d
}

fn set_tip(d: &mut NOTIFYICONDATAW, tip: &str) {
    let w: Vec<u16> = tip.encode_utf16().take(d.szTip.len() - 1).collect();
    d.szTip = [0; 128];
    d.szTip[..w.len()].copy_from_slice(&w);
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        CALLBACK => {
            match lparam as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => send(Ask::Show),
                WM_RBUTTONUP | WM_CONTEXTMENU => {
                    let menu = CreatePopupMenu();
                    let (show, quit) = (wide("Claude on the handheld…"), wide("Quit the bridge"));
                    AppendMenuW(menu, MF_STRING, MENU_SHOW, show.as_ptr());
                    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                    AppendMenuW(menu, MF_STRING, MENU_QUIT, quit.as_ptr());
                    let mut at = POINT { x: 0, y: 0 };
                    GetCursorPos(&mut at);
                    // So the menu closes when clicked away from.
                    SetForegroundWindow(hwnd);
                    let chosen = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, at.x, at.y, 0, hwnd, std::ptr::null());
                    DestroyMenu(menu);
                    match chosen as usize {
                        MENU_SHOW => send(Ask::Show),
                        MENU_QUIT => send(Ask::Quit),
                        _ => {}
                    }
                }
                _ => {}
            }
            0
        }
        SHOW => {
            send(Ask::Show);
            0
        }
        NOTE => {
            let mut d = data(hwnd);
            d.uFlags = NIF_INFO;
            d.dwInfoFlags = NIIF_INFO;
            let (title, text) = NOTE_TEXT.lock().unwrap_or_else(|e| e.into_inner()).clone();
            copy_into(&mut d.szInfoTitle, &title);
            copy_into(&mut d.szInfo, &text);
            Shell_NotifyIconW(NIM_MODIFY, &d);
            0
        }
        TIP => {
            let mut d = data(hwnd);
            d.uFlags = NIF_TIP;
            set_tip(&mut d, &TIP_TEXT.lock().unwrap_or_else(|e| e.into_inner()));
            Shell_NotifyIconW(NIM_MODIFY, &d);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// The tray's window handle, for its tooltip to be changed from elsewhere.
#[derive(Clone, Copy)]
pub struct Tray(usize);

impl Tray {
    pub fn tip(&self, tip: &str) {
        *TIP_TEXT.lock().unwrap_or_else(|e| e.into_inner()) = tip.to_owned();
        // SAFETY: posting to our own window, which lives while the tray does.
        unsafe { PostMessageW(self.0 as HWND, TIP, 0, 0) };
    }

    /// A balloon from the tray: what the bridge repaired or needs.
    pub fn notify(&self, title: &str, text: &str) {
        *NOTE_TEXT.lock().unwrap_or_else(|e| e.into_inner()) = (title.to_owned(), text.to_owned());
        // SAFETY: posting to our own window, which lives while the tray does.
        unsafe { PostMessageW(self.0 as HWND, NOTE, 0, 0) };
    }

    pub fn quit(&self) {
        // SAFETY: as above; WM_CLOSE ends the loop.
        unsafe { PostMessageW(self.0 as HWND, WM_CLOSE, 0, 0) };
    }
}

/// Put the bolt in the tray and run its messages until Quit (on this
/// thread): what it is asked goes to `asks`. `ready` gets its handle.
pub fn run(asks: Sender<Ask>, tip: &str, ready: impl FnOnce(Tray)) {
    *SENDER.lock().unwrap_or_else(|e| e.into_inner()) = Some(asks);
    let class = wide(CLASS);
    // SAFETY: the usual message-only window and its loop; strings live
    // across the calls that read them.
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: instance, lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0, HWND_MESSAGE, std::ptr::null_mut(), instance, std::ptr::null());
        let mut d = data(hwnd);
        d.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        d.uCallbackMessage = CALLBACK;
        d.hIcon = icon();
        set_tip(&mut d, tip);
        Shell_NotifyIconW(NIM_ADD, &d);
        ready(Tray(hwnd as usize));
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if msg.message == WM_CLOSE && msg.hwnd == hwnd {
                break;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        Shell_NotifyIconW(NIM_DELETE, &data(hwnd));
        DestroyWindow(hwnd);
    }
}
