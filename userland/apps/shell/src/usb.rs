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

/// Whether a computer has taken the handheld as its device: `None` where
/// there is no USB device controller (a computer, the simulator).
pub fn connected() -> Option<bool> {
    let dir = std::fs::read_dir("/sys/class/udc").ok()?;
    let udc = dir.flatten().next()?;
    let state = std::fs::read_to_string(udc.path().join("state")).ok()?;
    Some(state.trim() == "configured")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mode_is_kept_and_read_back() {
        crate::shell::tests::sandbox();
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
