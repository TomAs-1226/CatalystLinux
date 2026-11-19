//! `catalyst-update`: Catalyst Linux's system updates.
//!
//! On the handheld (the shell starts these; see update.rs for the whole story):
//!
//!     catalyst-update check      look for a newer release on the chosen channel
//!     catalyst-update install    download it and write it into the spare system
//!     catalyst-update apply      restart into it, once
//!     catalyst-update verify     at every start: keep a tried system that came up
//!     catalyst-update status     where an update stands, as JSON
//!
//! On the computer that makes releases (scripts/release.sh runs these):
//!
//!     catalyst-update keygen KEY PUBLIC
//!     catalyst-update make-release --images DIR --out DIR --version V --channel C
//!         --build EPOCH --key KEY --repo OWNER/REPO --tag TAG [--previous DIR] [--notes TEXT]

mod device;
mod manifest;
mod net;
mod pack;
mod release;
mod status;
mod update;

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(why) = run(&args) {
        eprintln!("catalyst-update: {why}");
        std::process::exit(1);
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let need = |name: &str| value(name).ok_or(format!("{name} is needed"));
    let dev = device::Device::here();
    match args.first().map(String::as_str) {
        Some("check") => update::check(&dev),
        Some("install") => update::install(&dev),
        Some("apply") => update::apply(&dev),
        Some("verify") => update::verify(&dev),
        Some("status") => {
            println!("{}", status::read(&dev).to_json());
            Ok(())
        }
        Some("keygen") => match (args.get(1), args.get(2)) {
            (Some(key), Some(public)) => release::keygen(&PathBuf::from(key), &PathBuf::from(public)),
            _ => Err("keygen KEY PUBLIC".into()),
        },
        Some("make-release") => {
            let key = std::fs::read_to_string(need("--key")?).map_err(|e| format!("the signing key: {e}"))?;
            let made = release::make(&release::Make {
                images: need("--images")?.into(),
                out: need("--out")?.into(),
                version: need("--version")?,
                channel: value("--channel").unwrap_or_else(|| "stable".into()),
                build: need("--build")?.parse().map_err(|_| "--build is seconds since 1970")?,
                notes: value("--notes").unwrap_or_default(),
                key: key.trim().to_owned(),
                previous: value("--previous").map(PathBuf::from),
                repo: need("--repo")?,
                tag: need("--tag")?,
            })?;
            println!("{} ({}): system {} MB, {} patch(es)", made.version, made.channel, made.full.size / 1_000_000, made.deltas.len());
            Ok(())
        }
        _ => Err("check | install | apply | verify | status | keygen | make-release".into()),
    }
}
