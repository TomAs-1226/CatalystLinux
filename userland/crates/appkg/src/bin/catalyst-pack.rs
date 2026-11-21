//! `catalyst-pack`: makes the app catalogue.
//!
//! ```text
//! catalyst-pack new ID [NAME] [--in DIR]   a script app to start from
//! catalyst-pack check DIR...               is each app fit for the catalogue?
//! catalyst-pack build SRC --out DIST --key KEY [--bin DIR] [--name NAME]
//!                                          every app under SRC, packed and
//!                                          signed, and DIST/index.json
//! catalyst-pack verify DIST --public FILE  every package against the index
//! catalyst-pack keygen KEY PUBLIC          the catalogue's signing key
//! ```
//!
//! An app whose app file says `binary = catalyst-sudoku` carries that
//! program, built for the handheld: `--bin` names the folder it was built
//! into (scripts/build-shell.sh puts them in ~/catalyst-linux-build/out/apps).

use std::path::{Path, PathBuf};

use appkg::{Entry, Listing, Manifest};

const HELP: &str = "catalyst-pack: makes the app catalogue

  catalyst-pack new ID [NAME] [--in DIR]
      a script app to start from, in DIR/ID
  catalyst-pack check DIR...
      whether each app is fit for the catalogue
  catalyst-pack build SRC --out DIST --key KEY [--bin DIR] [--name NAME]
      every app under SRC packed and signed, and DIST/index.json
  catalyst-pack verify DIST --public FILE
      every package in DIST against its index and the key
  catalyst-pack keygen KEY PUBLIC
      a new signing key, and its public half for the image";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("new") => new(&args[1..]),
        Some("check") => check(&args[1..]),
        Some("build") => build(&args[1..]),
        Some("verify") => verify(&args[1..]),
        Some("keygen") => keygen(&args[1..]),
        _ => {
            println!("{HELP}");
            Ok(())
        }
    };
    if let Err(why) = result {
        eprintln!("catalyst-pack: {why}");
        std::process::exit(1);
    }
}

/// The value after `--name`.
fn option<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

/// The arguments that are not options or their values.
fn plain(args: &[String]) -> Vec<&str> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
        } else if a.starts_with("--") {
            skip = true;
        } else {
            out.push(a.as_str());
        }
    }
    out
}

/// An app's folder read: its app file, with the id the folder gives it if
/// the file does not say.
fn read_app(dir: &Path) -> Result<Manifest, String> {
    let text = std::fs::read_to_string(dir.join("app")).map_err(|_| format!("{}: no app file", dir.display()))?;
    let mut m = Manifest::parse(&text);
    let folder = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if m.id.is_empty() {
        m.id = folder.clone();
    }
    if m.id != folder {
        return Err(format!("{}: its id is {} but its folder is {folder}", dir.display(), m.id));
    }
    Ok(m)
}

/// The path data of `icon.svg`, as the shell draws it: a Material Symbol in
/// its 960-unit box (the shell's own rules, in bezel::icons::svg_path).
fn icon_path(svg: &str) -> Option<String> {
    if !svg.contains("viewBox=\"0 -960 960 960\"") {
        return None;
    }
    let mut d = String::new();
    let mut rest = svg;
    while let Some(at) = rest.find(" d=\"") {
        rest = &rest[at + 4..];
        let end = rest.find('"')?;
        d.push_str(&rest[..end]);
        d.push(' ');
        rest = &rest[end..];
    }
    let fine = !d.trim().is_empty() && d.len() <= 16 * 1024 && d.chars().all(|c| c.is_ascii_alphanumeric() || " .,-+".contains(c));
    fine.then_some(d)
}

/// Everything wrong with an app folder for the catalogue.
fn problems(dir: &Path, m: &Manifest, bin: Option<&Path>) -> Vec<String> {
    let mut out = m.check().err().unwrap_or_default();
    if m.icon == "icon.svg" {
        match std::fs::read_to_string(dir.join("icon.svg")) {
            Ok(svg) if icon_path(&svg).is_some() => {}
            Ok(_) => out.push("icon.svg: a Material Symbol in its 960-unit box (fonts.gstatic.com's 20px.svg)".into()),
            Err(_) => out.push("icon.svg: missing".into()),
        }
    }
    let has = |name: &str| dir.join(name).is_file();
    if !has("LICENSE") && !has("LICENCE") {
        out.push("LICENSE: the licence's text is missing".into());
    }
    if !m.binary.is_empty() {
        if let Some(bin) = bin {
            if !bin.join(&m.binary).is_file() {
                out.push(format!("binary: {} is not built in {}", m.binary, bin.display()));
            }
        }
    } else {
        let program = m.run.split_whitespace().next().unwrap_or("");
        if let Some(local) = program.strip_prefix("./") {
            if !has(local) {
                out.push(format!("run: {local} is not in the folder"));
            }
        }
    }
    out
}

fn check(args: &[String]) -> Result<(), String> {
    let dirs = plain(args);
    if dirs.is_empty() {
        return Err("check DIR...".into());
    }
    let mut failed = 0;
    for dir in dirs {
        let dir = Path::new(dir);
        let m = read_app(dir)?;
        let found = problems(dir, &m, None);
        if found.is_empty() {
            println!("{}: fit for the catalogue ({} {})", m.id, m.name, m.version);
        } else {
            failed += 1;
            println!("{}:", m.id);
            for p in found {
                println!("  {p}");
            }
        }
    }
    if failed > 0 {
        return Err(format!("{failed} not fit"));
    }
    Ok(())
}

fn build(args: &[String]) -> Result<(), String> {
    let src = plain(args).first().copied().ok_or("build SRC --out DIST --key KEY")?;
    let out = PathBuf::from(option(args, "--out").ok_or("--out DIST")?);
    let key = std::fs::read_to_string(option(args, "--key").ok_or("--key KEY")?).map_err(|e| format!("the key: {e}"))?;
    let bin = option(args, "--bin").map(PathBuf::from);
    let name = option(args, "--name").unwrap_or("Catalyst apps");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(src).map_err(|e| format!("{src}: {e}"))?.flatten().map(|e| e.path()).filter(|p| p.join("app").is_file()).collect();
    dirs.sort();
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    // What an earlier build left: only what this one writes stays.
    for old in std::fs::read_dir(&out).map_err(|e| e.to_string())?.flatten() {
        let n = old.file_name().to_string_lossy().into_owned();
        if n.ends_with(".tar.gz") || n == "index.json" {
            std::fs::remove_file(old.path()).ok();
        }
    }
    let mut listings = Vec::new();
    for dir in &dirs {
        let m = read_app(dir)?;
        let found = problems(dir, &m, bin.as_deref());
        if !found.is_empty() {
            return Err(format!("{} is not fit for the catalogue:\n  {}", m.id, found.join("\n  ")));
        }
        let mut entries = Vec::new();
        for (path, file) in appkg::files_of(dir).map_err(|e| e.to_string())? {
            let data = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let executable = appkg::runs(&m, &path);
            entries.push(Entry { path, data, executable });
        }
        if !m.binary.is_empty() {
            let built = bin.as_ref().ok_or_else(|| format!("{} carries {}: say where it was built with --bin", m.id, m.binary))?.join(&m.binary);
            let data = std::fs::read(&built).map_err(|e| format!("{}: {e}", built.display()))?;
            entries.push(Entry { path: m.binary.clone(), data, executable: true });
            entries.sort_by(|a, b| a.path.cmp(&b.path));
        }
        let package = appkg::pack(&entries)?;
        if package.len() as u64 > appkg::PACKAGE_MAX {
            return Err(format!("{} is {} bytes: the catalogue takes at most {}", m.id, package.len(), appkg::PACKAGE_MAX));
        }
        let file = format!("{}-{}.tar.gz", m.id, m.version);
        std::fs::write(out.join(&file), &package).map_err(|e| e.to_string())?;
        let svg = if m.icon == "icon.svg" { std::fs::read_to_string(dir.join("icon.svg")).ok().and_then(|s| icon_path(&s)).unwrap_or_default() } else { String::new() };
        let signature = appkg::sign(appkg::statement(&m.id, &m.version, &appkg::sha256_hex(&package)).as_bytes(), &key)?;
        let listing = Listing::of(&m, &svg, &file, &package, signature);
        println!("packed {file}: {} KB, {} files, signed by {}", package.len().div_ceil(1024), entries.len(), appkg::key_id(&appkg::public_of(&key)?));
        listings.push(listing);
    }
    std::fs::write(out.join("index.json"), appkg::index_json(name, &listings)).map_err(|e| e.to_string())?;
    println!("index.json lists {} apps", listings.len());
    Ok(())
}

fn verify(args: &[String]) -> Result<(), String> {
    let dist = PathBuf::from(plain(args).first().copied().ok_or("verify DIST --public FILE")?);
    let keys = appkg::trusted_keys(&[PathBuf::from(option(args, "--public").ok_or("--public FILE")?)]);
    if keys.is_empty() {
        return Err("no public key in that file".into());
    }
    let text = std::fs::read_to_string(dist.join("index.json")).map_err(|e| format!("index.json: {e}"))?;
    let index: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("index.json: {e}"))?;
    let apps = appkg::parse_index(&index, "https://local/index.json");
    let mut bad = 0;
    for app in &apps {
        let file = app.archive.rsplit('/').next().unwrap_or("");
        let verdict = match std::fs::read(dist.join(file)) {
            Err(e) => format!("missing ({e})"),
            Ok(bytes) if appkg::sha256_hex(&bytes) != app.sha256 => "its bytes are not the ones the index names".into(),
            Ok(_) if appkg::signed_by(&app.id, &app.version, &app.sha256, &app.signature, &keys).is_none() => "not signed by this key".into(),
            Ok(bytes) => match appkg::unpack(&bytes) {
                Ok(files) if files.iter().any(|(p, ..)| p == "app") => "good".into(),
                Ok(_) => "no app file inside".into(),
                Err(e) => format!("unreadable ({e})"),
            },
        };
        if verdict != "good" {
            bad += 1;
        }
        println!("{} {}: {verdict}", app.id, app.version);
    }
    if bad > 0 {
        return Err(format!("{bad} of {} failed", apps.len()));
    }
    Ok(())
}

fn keygen(args: &[String]) -> Result<(), String> {
    let [key, public] = plain(args)[..] else { return Err("keygen KEY PUBLIC".into()) };
    if Path::new(key).exists() {
        return Err(format!("{key} is there already: a new key would orphan every handheld that trusts the old one"));
    }
    let mut seed = [0u8; 32];
    use std::io::Read;
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut seed)).map_err(|_| "no /dev/urandom here: make the key in WSL or on Linux".to_string())?;
    let seed = appkg::hex(&seed);
    let public_hex = appkg::public_of(&seed)?;
    std::fs::write(key, format!("{seed}\n")).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(key, std::fs::Permissions::from_mode(0o600)).ok();
    }
    std::fs::write(public, format!("# The key Catalyst's app catalogue is signed with ({}).\n{public_hex}\n", appkg::key_id(&public_hex))).map_err(|e| e.to_string())?;
    println!("signing key in {key} (keep it secret, keep a copy); public half in {public}");
    Ok(())
}

/// A script app to start from: it asks with `catalyst-ui` and answers.
fn new(args: &[String]) -> Result<(), String> {
    let words = plain(args);
    let id = words.first().copied().ok_or("new ID [NAME]")?;
    if !appkg::valid_id(id) {
        return Err("an id is letters, digits and dashes".into());
    }
    let name = words.get(1).copied().unwrap_or(id);
    let dir = PathBuf::from(option(args, "--in").unwrap_or(".")).join(id);
    if dir.exists() {
        return Err(format!("{} is there already", dir.display()));
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let m = Manifest {
        id: id.into(),
        name: name.into(),
        icon: "apps".into(),
        about: "an app of my own".into(),
        version: "1.0.0".into(),
        run: "sh ./run.sh".into(),
        screen: appkg::Screen::Own,
        licence: "MIT".into(),
        ..Manifest::default()
    };
    std::fs::write(dir.join("app"), m.render()).map_err(|e| e.to_string())?;
    let script = format!(
        "#!/bin/sh\n# {name}: asks with the handheld's own screens (catalyst-ui) and answers.\n# Esc on any of them ends the app.\n\nwhile true; do\n\tchoice=$(catalyst-ui menu \"{name}\" \"front_hand:Say hello\" \"schedule:What time is it\") || exit 0\n\tcase \"$choice\" in\n\t\"Say hello\")\n\t\twho=$(catalyst-ui input \"{name}\" \"What is your name?\") || continue\n\t\tcatalyst-ui message \"{name}\" \"Hello, $who.\"\n\t\t;;\n\t\"What time is it\")\n\t\tcatalyst-ui message \"{name}\" \"It is $(date +%H:%M).\"\n\t\t;;\n\tesac\ndone\n"
    );
    std::fs::write(dir.join("run.sh"), script).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("LICENSE"), MIT.replace("{who}", name)).map_err(|e| e.to_string())?;
    println!("{} is ready: copy it to /data/apps on the handheld and it is on Home", dir.display());
    Ok(())
}

const MIT: &str = "MIT License

Copyright (c) {who}'s authors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the \"Software\"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
";
