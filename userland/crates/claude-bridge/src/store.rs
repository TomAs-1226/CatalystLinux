//! The pairing, kept: the handheld's key in the bridge's own folder in the
//! user's profile (`%APPDATA%\Catalyst Companion\claude\pairing.json`),
//! encrypted to the Windows account with DPAPI. Never in the repository,
//! never on a command line, never in a log. Unpairing deletes it.

use std::path::Path;

use catalyst_companion_proto::claude::Key;
use serde_json::{json, Value};

/// The handheld paired: its key, its name, and since when.
#[derive(Clone, Debug, PartialEq)]
pub struct Paired {
    pub key: Key,
    pub handheld: String,
    pub since: i64,
}

fn file(dir: &Path) -> std::path::PathBuf {
    dir.join("pairing.json")
}

pub fn load(dir: &Path, protected: bool) -> Option<Paired> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(file(dir)).ok()?).ok()?;
    let sealed = catalyst_companion_proto::unhex(v["key"].as_str()?)?;
    let bytes = zeroize::Zeroizing::new(if protected { unprotect(&sealed)? } else { sealed });
    let key: [u8; 32] = bytes[..].try_into().ok()?;
    Some(Paired { key: Key::from_bytes(key), handheld: v["handheld"].as_str().unwrap_or("handheld").to_owned(), since: v["since"].as_i64().unwrap_or(0) })
}

pub fn save(dir: &Path, protected: bool, p: &Paired) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let bytes = if protected { protect(p.key.bytes()).ok_or("Windows would not encrypt the key")? } else { p.key.bytes().to_vec() };
    let v = json!({"key": catalyst_companion_proto::hex(&bytes), "handheld": p.handheld, "since": p.since});
    let path = file(dir);
    let temp = path.with_extension("saving");
    std::fs::write(&temp, v.to_string()).map_err(|e| e.to_string())?;
    restrict(&temp);
    std::fs::rename(&temp, &path).map_err(|e| e.to_string())
}

pub fn forget(dir: &Path) {
    std::fs::remove_file(file(dir)).ok();
}

/// Readable by this user alone where the system says so (a file in the
/// Windows profile already is).
pub fn restrict(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).ok();
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// DPAPI: bytes only this Windows account can open again.
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
