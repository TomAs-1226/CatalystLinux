//! Windows' own pieces: files dropped on the window, the open-file dialog,
//! the clipboard, DPAPI (the handheld's token is kept encrypted to this
//! Windows account), opening a folder or a link, and programs started with
//! no console window. Elsewhere (the tests on Linux), stand-ins.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

/// Where the app keeps what it knows (the handhelds paired).
pub fn app_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Catalyst Companion")
}

/// The sign-in window's own browser profile, apart from every other.
pub fn webview_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Catalyst Companion").join("YouTube Music")
}

/// Where downloads from the handheld go.
pub fn downloads_dir() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join("Downloads").join("Catalyst")
}

/// Where backups go.
pub fn backups_dir() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join("Documents").join("Catalyst backups")
}

/// This computer's name, as the handheld will list it.
pub fn computer_name() -> String {
    std::env::var("COMPUTERNAME").ok().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "This computer".into())
}

/// This computer's IPv4 addresses, one for each network adapter that is up
/// (asked of Windows directly: no name lookup to wait for).
pub fn local_ipv4() -> Vec<std::net::Ipv4Addr> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::NetworkManagement::IpHelper::{GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH};
        use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};
        let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
        let mut size: u32 = 16 * 1024;
        let mut buf: Vec<u64> = Vec::new();
        for _ in 0..3 {
            buf = vec![0u64; size as usize / 8 + 1];
            // SAFETY: `buf` holds `size` bytes, aligned for the structures written into it.
            let rc = unsafe { GetAdaptersAddresses(u32::from(AF_INET), flags, std::ptr::null(), buf.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>(), &mut size) };
            match rc {
                0 => break,
                111 => continue, // ERROR_BUFFER_OVERFLOW: `size` now says how much.
                _ => return Vec::new(),
            }
        }
        let mut out = Vec::new();
        let mut adapter = buf.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        // SAFETY: a list Windows wrote into `buf`, each pointer into it or null.
        unsafe {
            while !adapter.is_null() {
                let a = &*adapter;
                if a.OperStatus == 1 {
                    let mut u = a.FirstUnicastAddress;
                    while !u.is_null() {
                        let sa = (*u).Address.lpSockaddr;
                        if !sa.is_null() && (*sa).sa_family == AF_INET {
                            let sin = &*sa.cast::<SOCKADDR_IN>();
                            out.push(std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.S_un.S_addr)));
                        }
                        u = (*u).Next;
                    }
                }
                adapter = a.Next;
            }
        }
        out
    }
    #[cfg(not(windows))]
    Vec::new()
}

/// A program, with no console window of its own flashing up.
pub fn command(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// A folder opened in Explorer, `path` chosen in it if it is a file.
pub fn show_in_folder(path: &Path) {
    if path.is_file() {
        Command::new("explorer.exe").arg(format!("/select,{}", path.display())).spawn().ok();
    } else {
        Command::new("explorer.exe").arg(path).spawn().ok();
    }
}

/// A link opened in the browser.
pub fn open_url(url: &str) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let (verb, file) = (wide("open"), wide(url));
        // SAFETY: both strings are NUL-terminated and outlive the call.
        unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), 1) };
    }
    #[cfg(not(windows))]
    {
        let _ = url;
    }
}

/// `bytes` encrypted to this Windows account (DPAPI); elsewhere, as they are.
pub fn protect(bytes: &[u8]) -> Option<Vec<u8>> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB};
        let input = CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        // SAFETY: `input` points at `bytes` for the call; `out` is freed below.
        let ok = unsafe { CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0x1, &mut out) };
        if ok == 0 {
            return None;
        }
        // SAFETY: DPAPI gave `cbData` bytes at `pbData`.
        let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
        unsafe { LocalFree(out.pbData as _) };
        Some(v)
    }
    #[cfg(not(windows))]
    Some(bytes.to_vec())
}

pub fn unprotect(bytes: &[u8]) -> Option<Vec<u8>> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};
        let input = CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        // SAFETY: as in `protect`.
        let ok = unsafe { CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0x1, &mut out) };
        if ok == 0 {
            return None;
        }
        let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
        unsafe { LocalFree(out.pbData as _) };
        Some(v)
    }
    #[cfg(not(windows))]
    Some(bytes.to_vec())
}

/// The clipboard's text, if it holds any.
pub fn clipboard_text() -> Option<String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
        use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};
        const CF_UNICODETEXT: u32 = 13;
        // SAFETY: the clipboard is opened, read while locked, and closed.
        unsafe {
            if OpenClipboard(std::ptr::null_mut()) == 0 {
                return None;
            }
            let handle = GetClipboardData(CF_UNICODETEXT);
            let mut text = None;
            if !handle.is_null() {
                let p = GlobalLock(handle) as *const u16;
                if !p.is_null() {
                    let mut n = 0;
                    while *p.add(n) != 0 && n < 1 << 20 {
                        n += 1;
                    }
                    text = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)));
                    GlobalUnlock(handle);
                }
            }
            CloseClipboard();
            text
        }
    }
    #[cfg(not(windows))]
    None
}

/// Files chosen in Windows' open dialog (several at once when `many`).
/// `filter`: pairs of what to call them and their patterns (`*.mp3;*.flac`).
pub fn choose_files(window: *mut std::ffi::c_void, title: &str, filter: &[(&str, &str)], many: bool) -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST, OPENFILENAMEW};
        let mut filter_w: Vec<u16> = Vec::new();
        for (name, pattern) in filter {
            filter_w.extend(name.encode_utf16().chain(Some(0)));
            filter_w.extend(pattern.encode_utf16().chain(Some(0)));
        }
        filter_w.push(0);
        let title_w: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        let mut buf = vec![0u16; 64 * 1024];
        // SAFETY: a zeroed OPENFILENAMEW is valid; every pointer set outlives the call.
        let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
        ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        ofn.hwndOwner = window as _;
        ofn.lpstrFilter = filter_w.as_ptr();
        ofn.lpstrFile = buf.as_mut_ptr();
        ofn.nMaxFile = buf.len() as u32;
        ofn.lpstrTitle = title_w.as_ptr();
        ofn.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | if many { OFN_ALLOWMULTISELECT } else { 0 };
        if unsafe { GetOpenFileNameW(&mut ofn) } == 0 {
            return Vec::new();
        }
        // One file: its whole path. Several: the folder, then each name.
        let parts: Vec<String> = buf.split(|&c| c == 0).take_while(|p| !p.is_empty()).map(String::from_utf16_lossy).collect();
        match parts.as_slice() {
            [] => Vec::new(),
            [one] => vec![PathBuf::from(one)],
            [dir, names @ ..] => names.iter().map(|n| Path::new(dir).join(n)).collect(),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (window, title, filter, many);
        Vec::new()
    }
}

/// Files dropped on the window since last asked.
static DROPPED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

pub fn take_dropped() -> Vec<PathBuf> {
    std::mem::take(&mut *DROPPED.lock().unwrap_or_else(|e| e.into_inner()))
}

/// The window takes files dragged from Explorer.
pub fn accept_drops(window: *mut std::ffi::c_void) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
        use windows_sys::Win32::UI::Shell::{DefSubclassProc, DragAcceptFiles, DragFinish, DragQueryFileW, SetWindowSubclass, HDROP};
        const WM_DROPFILES: u32 = 0x0233;
        unsafe extern "system" fn on_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
            if msg == WM_DROPFILES {
                let drop = wparam as HDROP;
                // SAFETY: Windows gave this HDROP with the message; it is finished below.
                unsafe {
                    let count = DragQueryFileW(drop, u32::MAX, std::ptr::null_mut(), 0);
                    let mut got = Vec::new();
                    for i in 0..count {
                        let len = DragQueryFileW(drop, i, std::ptr::null_mut(), 0) as usize;
                        let mut buf = vec![0u16; len + 1];
                        DragQueryFileW(drop, i, buf.as_mut_ptr(), buf.len() as u32);
                        got.push(PathBuf::from(String::from_utf16_lossy(&buf[..len])));
                    }
                    DragFinish(drop);
                    DROPPED.lock().unwrap_or_else(|e| e.into_inner()).extend(got);
                }
                return 0;
            }
            // SAFETY: everything else goes on to the window as before.
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }
        // SAFETY: the window is minifb's, alive for the program's life.
        unsafe {
            DragAcceptFiles(window as HWND, 1);
            SetWindowSubclass(window as HWND, Some(on_message), 1, 0);
        }
    }
    #[cfg(not(windows))]
    let _ = window;
}
