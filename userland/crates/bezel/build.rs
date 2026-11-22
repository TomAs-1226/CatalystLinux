//! Turns `assets/icons/*.svg` (Material Symbols Rounded, 20 px optical size)
//! into a sorted table of SVG path strings the renderer rasterizes on demand.
//!
//! With the `embed-cjk` feature (the packed simulator, one file on a
//! computer with nothing installed), the CJK font is built in, if
//! scripts/cjk-font.py has made it; without it, CJK draws as boxes.

use std::{env, fs, path::Path};

fn main() {
    println!("cargo::rustc-check-cfg=cfg(cjk_embedded)");
    if env::var_os("CARGO_FEATURE_EMBED_CJK").is_some() {
        let font = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../assets/fonts/cjk/NotoSansCJK-Catalyst.otf");
        println!("cargo:rerun-if-changed={}", font.display());
        if font.is_file() {
            println!("cargo:rustc-cfg=cjk_embedded");
            println!("cargo:rustc-env=CATALYST_CJK_EMBEDDED={}", font.canonicalize().unwrap_or(font).display());
        } else {
            println!("cargo:warning=no CJK font to build in at {}: scripts/cjk-font.py makes it; until then CJK draws as boxes", font.display());
        }
    }

    let dir = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../assets/icons");
    println!("cargo:rerun-if-changed={}", dir.display());

    let mut icons: Vec<(String, String)> = Vec::new();
    for entry in fs::read_dir(&dir).expect("assets/icons") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("svg") {
            continue;
        }
        let svg = fs::read_to_string(&path).unwrap();
        // The renderer draws every symbol in Material Symbols' 960-unit box.
        // An older 20-unit file (as GitHub's copy of a few symbols still is)
        // would draw 48 times too small: take it from Google Fonts instead.
        assert!(
            svg.contains("viewBox=\"0 -960 960 960\""),
            "{} is not drawn in the 960-unit box: fetch it from fonts.gstatic.com/s/i/short-term/release/materialsymbolsrounded/<name>/default/20px.svg",
            path.display()
        );
        let mut d = String::new();
        let mut rest = svg.as_str();
        while let Some(at) = rest.find(" d=\"") {
            rest = &rest[at + 4..];
            let end = rest.find('"').expect("unterminated path");
            d.push_str(&rest[..end]);
            d.push(' ');
            rest = &rest[end..];
        }
        assert!(!d.is_empty(), "no path in {}", path.display());
        icons.push((path.file_stem().unwrap().to_str().unwrap().to_owned(), d));
    }
    icons.sort();

    let mut out = String::from("pub static ICONS: &[(&str, &str)] = &[\n");
    for (name, d) in &icons {
        out.push_str(&format!("    ({name:?}, {d:?}),\n"));
    }
    out.push_str("];\n");
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("icons.rs"), out).unwrap();
}
