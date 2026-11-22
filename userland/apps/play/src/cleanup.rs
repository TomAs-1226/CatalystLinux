//! Files the player leaves in /run while it plays (the head pose's socket,
//! what it plays and how), taken away as it ends: at the end of a song,
//! and when the shell stops it (SIGTERM, as a song is paused), so nobody
//! reads a player that is not there. The signal's handler only unlinks
//! and exits: nothing else is safe in one.

use std::ffi::CString;
use std::path::Path;
use std::sync::Mutex;

const MOST: usize = 4;

static PATHS: Mutex<Vec<CString>> = Mutex::new(Vec::new());
/// A copy the handler reads without a lock: pointers to the strings above,
/// which live until the process ends.
static mut RAW: [*const libc_char; MOST] = [std::ptr::null(); MOST];

#[allow(non_camel_case_types)]
type libc_char = std::ffi::c_char;

pub fn remove_at_exit(path: &Path) {
    let Ok(c) = CString::new(path.to_string_lossy().as_bytes()) else { return };
    let mut paths = PATHS.lock().unwrap();
    if paths.iter().any(|p| *p == c) || paths.len() >= MOST {
        return;
    }
    paths.push(c);
    // The strings' buffers do not move as the Vec grows (each is its own
    // allocation), and none is ever dropped.
    unsafe {
        for (i, p) in paths.iter().enumerate() {
            RAW[i] = p.as_ptr();
        }
    }
    install();
}

/// The files taken away now (a normal end).
pub fn now() {
    for p in PATHS.lock().unwrap().iter() {
        std::fs::remove_file(p.to_string_lossy().as_ref()).ok();
    }
}

#[cfg(target_os = "linux")]
fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        let handler = on_term as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::signal(libc::SIGTERM, handler);
        libc::signal(libc::SIGINT, handler);
    });
}

#[cfg(not(target_os = "linux"))]
fn install() {}

#[cfg(target_os = "linux")]
extern "C" fn on_term(sig: libc::c_int) {
    unsafe {
        #[allow(static_mut_refs)]
        for p in RAW.iter() {
            if !p.is_null() {
                libc::unlink(*p);
            }
        }
        libc::_exit(128 + sig);
    }
}
