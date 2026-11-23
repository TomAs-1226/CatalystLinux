//! The handheld as an update sees it: its two systems, which one is running,
//! whether this start is the one-time try of the other, and the boot
//! partition's `config.txt` that says which starts.
//!
//! `CATALYST_UPDATE_ROOT` puts all of it in a folder of plain files instead
//! (the systems as image files, the boot partition as a folder), for tests
//! and for the emulator.

use std::path::{Path, PathBuf};

/// The two systems: A and B.
pub const SLOTS: [&str; 2] = ["a", "b"];

pub struct Device {
    /// True when everything is plain files under a test folder.
    pub test: bool,
    pub boot_dev: PathBuf,
    /// Where the boot partition is (or is mounted, while an update writes it).
    pub boot_dir: PathBuf,
    /// System A's partition and system B's.
    pub systems: [PathBuf; 2],
    /// Downloads, status, preferences: on /data, so they survive a restart.
    pub state: PathBuf,
    /// The image's own settings: version, update source, public key.
    pub etc: PathBuf,
    pub cmdline: PathBuf,
    /// Set by the firmware when this start is a try (`reboot "0 tryboot"`).
    pub tryboot: PathBuf,
    /// Written by the shell once it has been up and drawing for a while.
    pub shell_ok: PathBuf,
    pub watchdog: PathBuf,
}

impl Device {
    pub fn here() -> Device {
        match std::env::var_os("CATALYST_UPDATE_ROOT") {
            Some(root) => Device::under(Path::new(&root)),
            None => Device {
                test: false,
                boot_dev: "/dev/mmcblk0p1".into(),
                boot_dir: "/boot".into(),
                systems: ["/dev/mmcblk0p2".into(), "/dev/mmcblk0p3".into()],
                state: "/data/update".into(),
                etc: "/etc/catalyst".into(),
                cmdline: "/proc/cmdline".into(),
                tryboot: "/proc/device-tree/chosen/bootloader/tryboot".into(),
                shell_ok: "/run/catalyst-shell-ok".into(),
                watchdog: "/dev/watchdog".into(),
            },
        }
    }

    /// A pretend handheld in a folder.
    pub fn under(root: &Path) -> Device {
        Device {
            test: true,
            boot_dev: root.join("boot.dev"),
            boot_dir: root.join("boot"),
            systems: [root.join("system-a.img"), root.join("system-b.img")],
            state: root.join("data/update"),
            etc: root.join("etc/catalyst"),
            cmdline: root.join("proc/cmdline"),
            tryboot: root.join("proc/tryboot"),
            shell_ok: root.join("run/catalyst-shell-ok"),
            watchdog: root.join("dev/watchdog"),
        }
    }

    /// The system running now: 0 for A, 1 for B, from the kernel's command line.
    pub fn running(&self) -> Result<usize, String> {
        let cmdline = std::fs::read_to_string(&self.cmdline).map_err(|e| format!("reading {}: {e}", self.cmdline.display()))?;
        slot_of_cmdline(&cmdline).ok_or_else(|| format!("no system partition in the command line: {}", cmdline.trim()))
    }

    /// Whether this start is the firmware's one-time try of the other system.
    pub fn tried(&self) -> bool {
        match std::fs::read(&self.tryboot) {
            // The device tree holds a 32-bit big-endian number; tests write "1".
            Ok(bytes) if bytes.len() == 4 => u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) == 1,
            Ok(bytes) => String::from_utf8_lossy(&bytes).trim() == "1",
            Err(_) => false,
        }
    }

    /// This system's version and when it was built (seconds since 1970).
    pub fn version(&self) -> (String, u64) {
        let text = std::fs::read_to_string(self.etc.join("version")).unwrap_or_default();
        let field = |key: &str| text.lines().find_map(|l| l.strip_prefix(key).and_then(|v| v.strip_prefix('='))).map(|v| v.trim().to_owned());
        (field("version").unwrap_or_else(|| "dev".into()), field("build").and_then(|b| b.parse().ok()).unwrap_or(0))
    }

    /// Where updates come from: `github:owner/repo`, or `url:<base>` for a
    /// plain folder of release files (a test server, or a mirror).
    pub fn source(&self) -> String {
        let read = |p: PathBuf| std::fs::read_to_string(p).ok().and_then(|t| t.lines().find_map(|l| l.strip_prefix("source=").map(|v| v.trim().to_owned())));
        read(self.state.join("prefs")).or_else(|| read(self.etc.join("update.conf"))).unwrap_or_default()
    }

    /// The key releases are signed with, as hex.
    pub fn public_key(&self) -> Option<String> {
        std::fs::read_to_string(self.etc.join("update.pub")).ok().map(|k| k.trim().to_owned()).filter(|k| !k.is_empty())
    }

    /// Make the boot partition writable while `f` runs, then put it back.
    pub fn with_boot<T>(&self, f: impl FnOnce(&Path) -> Result<T, String>) -> Result<T, String> {
        if self.test {
            std::fs::create_dir_all(&self.boot_dir).map_err(|e| e.to_string())?;
            return f(&self.boot_dir);
        }
        std::fs::create_dir_all(&self.boot_dir).ok();
        let mounted = std::fs::read_to_string("/proc/mounts").unwrap_or_default().lines().any(|l| l.split(' ').nth(1) == Some("/boot"));
        let ok = |status: std::io::Result<std::process::ExitStatus>| status.is_ok_and(|s| s.success());
        let up = if mounted {
            ok(std::process::Command::new("mount").args(["-o", "remount,rw", "/boot"]).status())
        } else {
            ok(std::process::Command::new("mount").args(["-t", "vfat", "-o", "rw"]).arg(&self.boot_dev).arg(&self.boot_dir).status())
        };
        if !up {
            return Err("could not open the boot partition for writing".into());
        }
        let result = f(&self.boot_dir);
        sync();
        if mounted {
            std::process::Command::new("mount").args(["-o", "remount,ro", "/boot"]).status().ok();
        } else {
            std::process::Command::new("umount").arg(&self.boot_dir).status().ok();
        }
        result
    }
}

/// Which system a kernel command line starts: its root partition, 2 for A
/// and 3 for B.
pub fn slot_of_cmdline(cmdline: &str) -> Option<usize> {
    cmdline.split_whitespace().find_map(|arg| match arg.strip_prefix("root=")? {
        "/dev/mmcblk0p2" => Some(0),
        "/dev/mmcblk0p3" => Some(1),
        _ => None,
    })
}

/// A system's kernel command line as it is installed: its own root, and
/// with the Grove port the smart camera's (Settings › Camera › Grove port,
/// catalyst-grove), no console on the port: `console=null` (the kernel's
/// ttynull, which keeps nothing) for `console=ttyAMA0…` or `console=serial0…`,
/// so the next system's boot log never reaches the camera either.
pub fn kernel_args(line: &str, root: &str, grove_camera: bool) -> String {
    line.split_whitespace()
        .map(|a| {
            if a.starts_with("root=") {
                format!("root={root}")
            } else if grove_camera && (a.starts_with("console=ttyAMA0") || a.starts_with("console=serial0")) {
                "console=null".to_owned()
            } else {
                a.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl Device {
    /// Whether the Grove port is the smart camera's (catalyst-grove keeps it
    /// beside the updater's own state on /data).
    pub fn grove_camera(&self) -> bool {
        let file = self.state.parent().map(|d| d.join("catalyst/grove-port"));
        file.and_then(|f| std::fs::read_to_string(f).ok()).is_some_and(|m| m.trim() == "camera")
    }
}

/// The partition a system's root is on.
pub fn root_of(slot: usize) -> &'static str {
    ["/dev/mmcblk0p2", "/dev/mmcblk0p3"][slot]
}

/// The boot partition's folder for a system.
pub fn slot_dir(slot: usize) -> String {
    format!("slot-{}", SLOTS[slot])
}

const SLOT_BLOCK: &str = "# Which system starts: written by catalyst-update.";

/// `config.txt` with its last block saying that `run` starts, and that
/// `try_slot` is the one tried once after `reboot "0 tryboot"`. Everything
/// above the block (the board's own settings) is kept as it is.
pub fn set_slots(config: &str, run: usize, try_slot: usize) -> String {
    let board = match config.find(SLOT_BLOCK) {
        Some(at) => &config[..at],
        None => config,
    };
    format!(
        "{}\n\n{SLOT_BLOCK} [all] is the system in\n# use; [tryboot] the one an update is tried in, once, after reboot \"0 tryboot\".\n[all]\nos_prefix={}/\n[tryboot]\nos_prefix={}/\n",
        board.trim_end(),
        slot_dir(run),
        slot_dir(try_slot)
    )
}

/// Write which system starts: `config.txt` starts `run` (and, for firmware
/// that reads it on a try, tries `try_slot` in its `[tryboot]` section), and
/// `tryboot.txt` starts `try_slot`. Raspberry Pi firmware for this chip
/// (bootcode.bin and start.elf for the BCM2837/RP3A0) reads `tryboot.txt`
/// *instead of* `config.txt` on a start after `reboot "0 tryboot"`: without
/// it, the try found no settings and the old system came back (0.5.0's first
/// update on a CardputerZero). Each is written whole and renamed into place.
pub fn write_slots(boot: &Path, run: usize, try_slot: usize) -> Result<(), String> {
    let config_path = boot.join("config.txt");
    let config = std::fs::read_to_string(&config_path).map_err(|e| format!("reading config.txt: {e}"))?;
    let put = |name: &str, text: String| -> Result<(), String> {
        let tmp = boot.join(format!("{name}.new"));
        std::fs::write(&tmp, text).map_err(|e| format!("writing {name}: {e}"))?;
        std::fs::rename(&tmp, boot.join(name)).map_err(|e| format!("writing {name}: {e}"))
    };
    // The trial's settings first: a config.txt asking for a try is never
    // left without the file the try reads.
    put("tryboot.txt", set_slots(&config, try_slot, try_slot))?;
    put("config.txt", set_slots(&config, run, try_slot))
}

/// The system `config.txt` starts normally, and the one it tries.
#[cfg(test)]
pub fn slots_of(config: &str) -> Option<(usize, usize)> {
    let block = &config[config.find(SLOT_BLOCK)?..];
    let mut section = "";
    let (mut run, mut tried) = (None, None);
    for line in block.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line;
        } else if let Some(prefix) = line.strip_prefix("os_prefix=") {
            let slot = SLOTS.iter().position(|s| prefix.trim_end_matches('/') == format!("slot-{s}"));
            match section {
                "[all]" => run = slot,
                "[tryboot]" => tried = slot,
                _ => {}
            }
        }
    }
    Some((run?, tried?))
}

pub fn sync() {
    #[cfg(target_os = "linux")]
    unsafe {
        libc::sync();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_running_system_is_read_from_the_command_line() {
        assert_eq!(slot_of_cmdline("console=tty1 root=/dev/mmcblk0p2 ro rootwait"), Some(0));
        assert_eq!(slot_of_cmdline("root=/dev/mmcblk0p3 panic=10"), Some(1));
        assert_eq!(slot_of_cmdline("root=/dev/sda1"), None);
    }

    #[test]
    fn the_grove_ports_choice_is_carried_into_the_next_system() {
        let shipped = "root=/dev/mmcblk0p2 rootfstype=ext4 ro rootwait console=ttyAMA0,115200 loglevel=3 cma=64M panic=10";
        // As the port's console: only the root changes.
        assert_eq!(kernel_args(shipped, "/dev/mmcblk0p3", false), shipped.replace("p2", "p3"));
        // As the smart camera's: no console on the port, whatever it was called.
        let camera = kernel_args(shipped, "/dev/mmcblk0p3", true);
        assert!(camera.contains("console=null") && !camera.contains("ttyAMA0") && camera.contains("root=/dev/mmcblk0p3"), "{camera}");
        assert_eq!(kernel_args("root=x console=serial0,115200 console=tty1", "/dev/mmcblk0p2", true), "root=/dev/mmcblk0p2 console=null console=tty1");
        // Read from /data beside the updater's state.
        let root = std::env::temp_dir().join(format!("catalyst-grove-update-{}", std::process::id()));
        let dev = Device::under(&root);
        std::fs::create_dir_all(root.join("data/catalyst")).unwrap();
        assert!(!dev.grove_camera());
        std::fs::write(root.join("data/catalyst/grove-port"), "camera\n").unwrap();
        assert!(dev.grove_camera());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn only_the_last_block_of_config_txt_changes() {
        let board = "kernel=Image\narm_64bit=1\ndtoverlay=dwc2\n";
        let first = set_slots(board, 0, 1);
        assert!(first.starts_with(board));
        assert_eq!(slots_of(&first), Some((0, 1)));
        // Swapping keeps the board's settings and writes one block, not two.
        let swapped = set_slots(&first, 1, 0);
        assert_eq!(slots_of(&swapped), Some((1, 0)));
        assert_eq!(swapped.matches(SLOT_BLOCK).count(), 1);
        assert!(swapped.starts_with(board));
        assert!(swapped.contains("[all]\nos_prefix=slot-b/\n[tryboot]\nos_prefix=slot-a/"));
    }

    #[test]
    fn the_developer_trials_lines_stay_through_an_update() {
        // catalyst-power puts its blocks just above the system block, at run
        // time on the handheld: an update moving between slots keeps them.
        let board = "kernel=Image\ndtoverlay=cardputerzero-v5-overlay\n\n# catalyst-power: lower voltage (Settings > Developer options)\nover_voltage=-1\nover_voltage_min=-2\n# catalyst-power: end\n\n# catalyst-power: tear-free panel (Settings > Developer options)\ndtoverlay=cardputerzero-portrait\n# catalyst-power: end\n";
        let first = set_slots(board, 0, 1);
        let swapped = set_slots(&first, 1, 0);
        for line in ["over_voltage=-1", "over_voltage_min=-2", "dtoverlay=cardputerzero-portrait"] {
            assert_eq!(swapped.matches(line).count(), 1, "{line}");
            // Above the slot block, so in force whichever system starts.
            assert!(swapped.find(line) < swapped.find(SLOT_BLOCK), "{line}");
        }
    }

    #[test]
    fn a_try_is_read_from_the_device_tree() {
        let dir = std::env::temp_dir().join(format!("catalyst-update-tryboot-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("proc")).unwrap();
        let device = Device::under(&dir);
        assert!(!device.tried(), "no flag: a normal start");
        std::fs::write(&device.tryboot, [0, 0, 0, 1]).unwrap();
        assert!(device.tried());
        std::fs::write(&device.tryboot, [0, 0, 0, 0]).unwrap();
        assert!(!device.tried());
        std::fs::remove_dir_all(&dir).ok();
    }
}
