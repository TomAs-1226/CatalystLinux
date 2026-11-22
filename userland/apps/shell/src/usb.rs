//! Settings › USB: what the handheld is to a computer its USB-C port is
//! plugged into, through `/usr/sbin/catalyst-usb` (the kernel's USB gadget
//! framework): nothing, a serial console, a network adapter, or a read-only
//! drive of the match logs. The choice stays across restarts. In the
//! simulator it is only remembered.

/// Each mode: its name to catalyst-usb, what Settings calls it, and what the
/// computer sees (the page's bottom line).
pub const MODES: [(&str, &str, &str); 4] = [
    ("off", "charging only", "the computer only charges it"),
    ("serial", "serial console", "a serial port with a root login on it"),
    ("network", "network", "network adapter · this is 192.168.7.1"),
    ("drive", "drive of logs", "a read-only drive of the match logs"),
];

const SCRIPT: &str = "/usr/sbin/catalyst-usb";

/// Where the choice is kept: on the handheld, where catalyst-usb keeps it.
fn mode_file() -> std::path::PathBuf {
    if std::path::Path::new(SCRIPT).exists() {
        "/data/catalyst/usb-mode".into()
    } else {
        crate::config_dir().join("usb-mode")
    }
}

/// The mode chosen, by its place in `MODES`.
pub fn mode() -> usize {
    let saved = std::fs::read_to_string(mode_file()).unwrap_or_default();
    MODES.iter().position(|(name, ..)| *name == saved.trim()).unwrap_or(0)
}

/// Choose a mode. On the handheld it is set up in the background (a drive
/// of logs takes a moment to make); the page's status line follows.
pub fn set(index: usize) -> Result<(), String> {
    let (name, ..) = MODES[index % MODES.len()];
    if std::path::Path::new(SCRIPT).exists() {
        // Detached, so it belongs to no one and leaves no zombie.
        std::process::Command::new("sh")
            .args(["-c", &format!("{SCRIPT} {name} >/dev/null 2>&1 &")])
            .status()
            .map_err(|e| format!("usb: {e}"))?;
        Ok(())
    } else {
        std::fs::create_dir_all(crate::config_dir()).ok();
        crate::sys::write_atomic(&mode_file(), name.as_bytes()).map_err(|e| format!("usb: {e}"))
    }
}

/// A mode a computer sees is chosen, but the USB controller is still in the
/// role it started in (the host, for the internal hub): it takes the device
/// role from the next start (catalyst-usb sets it in config.txt), with the
/// USB switch to the left.
pub fn restart_needed() -> bool {
    mode() != 0 && std::path::Path::new(SCRIPT).exists() && !running_as_device()
}

/// The USB controller started in the device role (`dr_mode` peripheral).
pub fn running_as_device() -> bool {
    std::fs::read_dir("/proc/device-tree/soc")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("usb@"))
        .find_map(|e| std::fs::read_to_string(e.path().join("dr_mode")).ok())
        .is_some_and(|s| s.trim_end_matches('\0').trim() == "peripheral")
}

/// Whether a computer has taken the handheld as its device: `None` where
/// there is no USB device controller (a computer, the simulator).
pub fn connected() -> Option<bool> {
    let dir = std::fs::read_dir("/sys/class/udc").ok()?;
    let udc = dir.flatten().next()?;
    let state = std::fs::read_to_string(udc.path().join("state")).ok()?;
    Some(state.trim() == "configured")
}

/// The tests that set the mode take turns: the file it is kept in is one
/// for the whole test run.
#[cfg(test)]
pub(crate) static MODE_FILE: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mode_is_kept_and_read_back() {
        crate::shell::tests::sandbox();
        let _turn = MODE_FILE.lock().unwrap_or_else(|e| e.into_inner());
        set(2).unwrap();
        assert_eq!(MODES[mode()].0, "network");
        set(0).unwrap();
        assert_eq!(mode(), 0);
    }

    #[test]
    fn what_each_mode_shows_fits_the_page() {
        let mut g = crate::gfx::Gfx::new(bezel::Tokens::dark());
        for size in [crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let caption = g.ty.caption;
            for (_, _, sees) in MODES {
                assert!(g.measure(sees, caption) <= crate::gfx::W - 2.0 * crate::gfx::PAD, "{sees}");
            }
        }
    }
}
