//! On Windows, the program carries Catalyst's icon (the FrcCatalyst bolt), so
//! the simulator shows it in Explorer, on the taskbar and on its window.
//!
//! The icon goes in as a compiled resource file (`.res`) written here, which
//! Microsoft's linker takes as it is: no resource compiler is needed. Each size
//! is a PNG, as Windows has read icons since Vista.

use std::{env, fs, path::Path};

/// The sizes Windows asks for: 100% to 200% scaling of the small and large
/// icon, Explorer's views, and the jumbo one.
const SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

/// Resource types and the icon's name, which the shell loads its window's icon by.
const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const ICON_NAME: u16 = 1;

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/app-icon");
    println!("cargo:rerun-if-changed={}", dir.display());
    let windows = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let msvc = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if !(windows && msvc) {
        return;
    }
    let images: Vec<(u32, Vec<u8>)> = SIZES.iter().map(|&n| (n, fs::read(dir.join(format!("icon-{n}.png"))).expect("an icon size"))).collect();

    let mut res = Vec::new();
    // A resource file opens with an empty entry.
    entry(&mut res, 0, 0, &[]);
    for (i, (_, png)) in images.iter().enumerate() {
        entry(&mut res, RT_ICON, i as u16 + 1, png);
    }
    // The group: which image is which size.
    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(images.len() as u16).to_le_bytes());
    for (i, (n, png)) in images.iter().enumerate() {
        let side = if *n >= 256 { 0 } else { *n as u8 };
        group.extend_from_slice(&[side, side, 0, 0]);
        group.extend_from_slice(&1u16.to_le_bytes());
        group.extend_from_slice(&32u16.to_le_bytes());
        group.extend_from_slice(&(png.len() as u32).to_le_bytes());
        group.extend_from_slice(&(i as u16 + 1).to_le_bytes());
    }
    entry(&mut res, RT_GROUP_ICON, ICON_NAME, &group);

    let out = Path::new(&env::var("OUT_DIR").unwrap()).join("catalyst-icon.res");
    fs::write(&out, res).unwrap();
    println!("cargo:rustc-link-arg-bins={}", out.display());
}

/// One resource: a header naming its type and number, then its data, each
/// padded to four bytes.
fn entry(res: &mut Vec<u8>, kind: u16, name: u16, data: &[u8]) {
    let ordinal = |v: u16| [0xff, 0xff, v as u8, (v >> 8) as u8];
    let empty = kind == 0;
    res.extend_from_slice(&(data.len() as u32).to_le_bytes());
    res.extend_from_slice(&32u32.to_le_bytes());
    res.extend_from_slice(&ordinal(kind));
    res.extend_from_slice(&ordinal(name));
    res.extend_from_slice(&0u32.to_le_bytes()); // data version
    let (flags, language): (u16, u16) = if empty { (0, 0) } else { (0x1010, 0x0409) };
    res.extend_from_slice(&flags.to_le_bytes());
    res.extend_from_slice(&language.to_le_bytes());
    res.extend_from_slice(&0u32.to_le_bytes()); // version
    res.extend_from_slice(&0u32.to_le_bytes()); // characteristics
    res.extend_from_slice(data);
    while res.len() % 4 != 0 {
        res.push(0);
    }
}
