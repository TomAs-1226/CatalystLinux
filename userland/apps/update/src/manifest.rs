//! A release's manifest: what version it is, when it was built, and every
//! file of it with its size and SHA-256. The manifest is signed (Ed25519) by
//! the key kept on the computer that makes releases; the handheld holds the
//! public half and installs nothing whose manifest it cannot verify.

use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq)]
pub struct File {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

/// A patch from one version's system to this one's.
#[derive(Clone, Debug, PartialEq)]
pub struct Delta {
    pub from: String,
    /// The system it patches, exactly: checked before it is used.
    pub base_sha256: String,
    pub file: File,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub version: String,
    pub channel: String,
    /// When it was built: seconds since 1970. Newer builds are offered.
    pub build: u64,
    pub notes: String,
    /// The system image: its size and hash once written, the whole of it
    /// compressed, and patches from earlier versions.
    pub system_size: u64,
    pub system_sha256: String,
    pub full: File,
    pub deltas: Vec<Delta>,
    /// The kernel, device trees and overlays, for the boot partition.
    pub boot: File,
}

fn file_of(v: &Value) -> Option<File> {
    Some(File { name: v["name"].as_str()?.to_owned(), size: v["size"].as_u64()?, sha256: v["sha256"].as_str()?.to_owned() })
}

fn file_json(f: &File) -> Value {
    json!({ "name": f.name, "size": f.size, "sha256": f.sha256 })
}

impl Manifest {
    pub fn parse(bytes: &[u8]) -> Result<Manifest, String> {
        let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("the manifest is not JSON: {e}"))?;
        let bad = |what: &str| format!("the manifest has no {what}");
        let system = &v["system"];
        Ok(Manifest {
            version: v["version"].as_str().ok_or_else(|| bad("version"))?.to_owned(),
            channel: v["channel"].as_str().unwrap_or("stable").to_owned(),
            build: v["build"].as_u64().ok_or_else(|| bad("build time"))?,
            notes: v["notes"].as_str().unwrap_or_default().to_owned(),
            system_size: system["size"].as_u64().ok_or_else(|| bad("system size"))?,
            system_sha256: system["sha256"].as_str().ok_or_else(|| bad("system hash"))?.to_owned(),
            full: file_of(&system["full"]).ok_or_else(|| bad("full system file"))?,
            deltas: system["deltas"]
                .as_array()
                .map(|all| {
                    all.iter()
                        .filter_map(|d| Some(Delta { from: d["from"].as_str()?.to_owned(), base_sha256: d["base_sha256"].as_str()?.to_owned(), file: file_of(&d["file"])? }))
                        .collect()
                })
                .unwrap_or_default(),
            boot: file_of(&v["boot"]).ok_or_else(|| bad("boot file"))?,
        })
    }

    pub fn to_json(&self) -> String {
        let v = json!({
            "version": self.version,
            "channel": self.channel,
            "build": self.build,
            "notes": self.notes,
            "system": {
                "size": self.system_size,
                "sha256": self.system_sha256,
                "full": file_json(&self.full),
                "deltas": self.deltas.iter().map(|d| json!({ "from": d.from, "base_sha256": d.base_sha256, "file": file_json(&d.file) })).collect::<Vec<_>>(),
            },
            "boot": file_json(&self.boot),
        });
        serde_json::to_string_pretty(&v).expect("a manifest is always JSON")
    }

    /// The patch from `version` whose base is `base_sha256`, if this release has one.
    pub fn delta_from(&self, version: &str) -> Option<&Delta> {
        self.deltas.iter().find(|d| d.from == version)
    }
}

/// Check a manifest's signature against the public key (both hex).
pub fn verify(manifest: &[u8], signature_hex: &str, public_hex: &str) -> Result<(), String> {
    let key = unhex(public_hex).ok_or("the update key is not hex")?;
    let key = ed25519_compact::PublicKey::from_slice(&key).map_err(|_| "the update key is not an Ed25519 key")?;
    let sig = unhex(signature_hex.trim()).ok_or("the signature is not hex")?;
    let sig = ed25519_compact::Signature::from_slice(&sig).map_err(|_| "the signature is not an Ed25519 signature")?;
    key.verify(manifest, &sig).map_err(|_| "the manifest's signature does not match: not installing it".into())
}

/// Sign a manifest with a secret key's 32-byte seed (hex).
pub fn sign(manifest: &[u8], seed_hex: &str) -> Result<String, String> {
    let pair = key_pair(seed_hex)?;
    Ok(hex(pair.sk.sign(manifest, None).as_ref()))
}

/// The public key (hex) for a secret key's seed.
pub fn public_of(seed_hex: &str) -> Result<String, String> {
    Ok(hex(key_pair(seed_hex)?.pk.as_ref()))
}

fn key_pair(seed_hex: &str) -> Result<ed25519_compact::KeyPair, String> {
    let seed = unhex(seed_hex.trim()).filter(|s| s.len() == 32).ok_or("the signing key should be 32 bytes of hex")?;
    let seed = ed25519_compact::Seed::from_slice(&seed).map_err(|_| "bad signing key")?;
    Ok(ed25519_compact::KeyPair::from_seed(seed))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        let file = |name: &str| File { name: name.into(), size: 10, sha256: "ab".repeat(32) };
        Manifest {
            version: "0.3.0".into(),
            channel: "stable".into(),
            build: 1_763_500_000,
            notes: "faster glass".into(),
            system_size: 167_772_160,
            system_sha256: "cd".repeat(32),
            full: file("system.img.gz"),
            deltas: vec![Delta { from: "0.2.0".into(), base_sha256: "ef".repeat(32), file: file("system-from-0.2.0.cdelta") }],
            boot: file("boot.pack"),
        }
    }

    #[test]
    fn a_manifest_reads_back_as_written() {
        let m = sample();
        assert_eq!(Manifest::parse(m.to_json().as_bytes()).unwrap(), m);
        assert_eq!(m.delta_from("0.2.0").map(|d| d.file.name.as_str()), Some("system-from-0.2.0.cdelta"));
        assert!(m.delta_from("0.1.0").is_none());
    }

    #[test]
    fn only_the_right_key_and_the_same_bytes_verify() {
        let seed = "11".repeat(32);
        let public = public_of(&seed).unwrap();
        let text = sample().to_json();
        let sig = sign(text.as_bytes(), &seed).unwrap();
        assert!(verify(text.as_bytes(), &sig, &public).is_ok());
        // One byte changed: refused.
        let tampered = text.replacen("0.3.0", "0.3.1", 1);
        assert!(verify(tampered.as_bytes(), &sig, &public).is_err());
        // Another key: refused.
        let other = public_of(&"22".repeat(32)).unwrap();
        assert!(verify(text.as_bytes(), &sig, &other).is_err());
    }
}
