//! Named places, from the Wi-Fi around a photo: offline, and kept on the
//! handheld alone.
//!
//! As a photo is taken, the strongest access points in sight are noted with
//! their signal (a fingerprint: wpa_supplicant's last scan, `scan_results`).
//! "Name this place" gives a fingerprint a name; a later photo whose
//! fingerprint is like one of a place's is that place, and so is every
//! photo taken there before it was named (`Book::apply_places`).
//!
//! **Alike.** Each access point weighs its signal over −100 dBm (−40 dBm:
//! 60; −90: 10), and two fingerprints are as alike as the weight they share
//! over the weight either has (the weighted Jaccard, or Ruzicka, similarity:
//! the sum of the smaller weights over the sum of the larger). An access
//! point's other radios (its 5 GHz beside its 2.4, a guest network beside
//! the team's) differ only in the last few bits of their address or in the
//! "locally administered" bit, so those are set aside first and the radios
//! count as one. A place is matched at [`SAME_PLACE`] or more, with two
//! access points in common at least; the threshold is tuned against
//! simulated scans (`tests.rs`: the same room scanned again, the room next
//! door, across town).
//!
//! **What travels is left out.** A robot's radio (its SSID the team number:
//! "5805" or "5805_Robot") and phones' hotspots go wherever the team goes,
//! so they would make the shop and the pit look alike: they are never
//! noted. Nor is any SSID: only addresses and signal are kept, and they
//! never leave the handheld unless the town lookup is turned on (and then
//! only those that may be: [`Ap::share`]).

/// One access point seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ap {
    /// Its address (BSSID), 48 bits.
    pub bssid: u64,
    /// Signal, dBm.
    pub dbm: i8,
    /// Whether it may be sent to a location service: not a hidden network,
    /// and not one whose owner asked not to be mapped (`_nomap`).
    pub share: bool,
}

/// The access points kept with a photo: the strongest this many.
pub const KEEP: usize = 12;

/// Fingerprints at least this alike are the same place (with two access
/// points in common). Tuned in tests.rs against simulated scans (300 of
/// each): the same room scanned again scores 0.61 at the lowest (0.75
/// typically); another day there, rougher and with its main router off,
/// 0.57 typically and over 0.42 in 99 of 100; the room next door, which
/// hears seven of the same access points, weakly, 0.32 at the highest;
/// across town, 0.
pub const SAME_PLACE: f32 = 0.42;

/// A BSSID as text: "aa:bb:cc:dd:ee:ff".
pub fn bssid_text(b: u64) -> String {
    (0..6).rev().map(|i| format!("{:02x}", (b >> (i * 8)) & 0xff)).collect::<Vec<_>>().join(":")
}

pub fn parse_bssid(text: &str) -> Option<u64> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    parts.iter().try_fold(0u64, |acc, p| Some((acc << 8) | u64::from(u8::from_str_radix(p, 16).ok()?)))
}

/// A network that goes where the team goes: a robot's radio (named for its
/// team number: "5805", "5805_Robot") or a phone's hotspot.
pub fn travels(ssid: &str) -> bool {
    let digits = ssid.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && digits <= 5 && (ssid.len() == digits || ssid[digits..].starts_with('_')) {
        return true;
    }
    let lower = ssid.to_lowercase();
    ["iphone", "android", "galaxy", "pixel", "hotspot", "direct-", "oneplus", "redmi", "moto "].iter().any(|w| lower.contains(w))
}

/// wpa_cli's `scan_results` (bssid, frequency, signal, flags, ssid, a tab
/// between each) as a fingerprint: what travels left out, the strongest
/// [`KEEP`] kept.
pub fn parse_scan(text: &str) -> Vec<Ap> {
    let mut seen: Vec<Ap> = Vec::new();
    for line in text.lines().skip(1) {
        let fields: Vec<&str> = line.split('\t').collect();
        let [bssid, _, signal, _flags, ssid] = fields[..] else {
            // A hidden network: no SSID at all.
            if fields.len() == 4 {
                if let (Some(b), Ok(s)) = (parse_bssid(fields[0]), fields[2].trim().parse::<i32>()) {
                    seen.push(Ap { bssid: b, dbm: s.clamp(-127, 0) as i8, share: false });
                }
            }
            continue;
        };
        let Some(bssid) = parse_bssid(bssid) else { continue };
        let Ok(dbm) = signal.trim().parse::<i32>() else { continue };
        let ssid = crate::wifi::unescape(ssid);
        if travels(&ssid) {
            continue;
        }
        let hidden = ssid.is_empty() || ssid.bytes().all(|b| b == 0);
        let share = !hidden && !ssid.to_lowercase().ends_with("_nomap");
        seen.push(Ap { bssid, dbm: dbm.clamp(-127, 0) as i8, share });
    }
    strongest(seen)
}

/// The strongest [`KEEP`], each address once.
pub fn strongest(mut aps: Vec<Ap>) -> Vec<Ap> {
    aps.sort_by(|a, b| b.dbm.cmp(&a.dbm).then(a.bssid.cmp(&b.bssid)));
    let mut out: Vec<Ap> = Vec::new();
    for ap in aps {
        if !out.iter().any(|o| o.bssid == ap.bssid) {
            out.push(ap);
        }
    }
    out.truncate(KEEP);
    out
}

/// One access point's radios as one: the locally administered bit and the
/// address's last four bits set aside.
fn radio(bssid: u64) -> u64 {
    bssid & !0x0200_0000_0000 & !0xf
}

fn weight(dbm: i8) -> f32 {
    (f32::from(dbm) + 100.0).clamp(1.0, 70.0)
}

/// Each radio once, at its strongest weight.
fn radios(fp: &[Ap]) -> Vec<(u64, f32)> {
    let mut out: Vec<(u64, f32)> = Vec::new();
    for ap in fp {
        let (r, w) = (radio(ap.bssid), weight(ap.dbm));
        match out.iter_mut().find(|(k, _)| *k == r) {
            Some(o) => o.1 = o.1.max(w),
            None => out.push((r, w)),
        }
    }
    out
}

/// How alike two fingerprints are, 0 to 1, and how many radios they share.
pub fn likeness(a: &[Ap], b: &[Ap]) -> (f32, usize) {
    let (ra, rb) = (radios(a), radios(b));
    let (mut both, mut either, mut common) = (0.0f32, 0.0f32, 0);
    for (k, wa) in &ra {
        match rb.iter().find(|(kb, _)| kb == k) {
            Some((_, wb)) => {
                both += wa.min(*wb);
                either += wa.max(*wb);
                common += 1;
            }
            None => either += wa,
        }
    }
    for (k, wb) in &rb {
        if !ra.iter().any(|(ka, _)| ka == k) {
            either += wb;
        }
    }
    (if either > 0.0 { both / either } else { 0.0 }, common)
}

/// Whether two fingerprints are of the same place.
pub fn same_place(a: &[Ap], b: &[Ap]) -> bool {
    let (score, common) = likeness(a, b);
    common >= 2 && score >= SAME_PLACE
}

/// A place with a name: the fingerprints it was named from.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub id: u32,
    pub name: String,
    pub samples: Vec<Vec<Ap>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Places {
    pub list: Vec<Place>,
}

/// Names are a few words: "Shop", "Home", "Pit".
pub const NAME_MOST: usize = 24;

/// A name as typed, tidied: one line, no ends, at most [`NAME_MOST`].
pub fn clean_name(typed: &str) -> String {
    let one: String = typed.split_whitespace().collect::<Vec<_>>().join(" ");
    one.chars().filter(|c| !c.is_control()).take(NAME_MOST).collect::<String>().trim().to_owned()
}

impl Places {
    /// The place a fingerprint is, if it is any: the one most alike.
    pub fn best(&self, fp: &[Ap]) -> Option<&Place> {
        let mut best: Option<(&Place, f32)> = None;
        for p in &self.list {
            for s in &p.samples {
                let (score, common) = likeness(fp, s);
                if common >= 2 && score >= SAME_PLACE && best.is_none_or(|(_, b)| score > b) {
                    best = Some((p, score));
                }
            }
        }
        best.map(|(p, _)| p)
    }

    pub fn get(&self, id: u32) -> Option<&Place> {
        self.list.iter().find(|p| p.id == id)
    }

    pub fn by_name(&self, name: &str) -> Option<&Place> {
        self.list.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// `fp` is `name`: a new place, or another look at one already named
    /// so (its fingerprint kept beside the others). Its id.
    pub fn name(&mut self, name: &str, fp: &[Ap]) -> Option<u32> {
        let name = clean_name(name);
        if name.is_empty() || fp.is_empty() {
            return None;
        }
        if let Some(id) = self.by_name(&name).map(|p| p.id) {
            self.add_sample(id, fp);
            return Some(id);
        }
        let id = self.list.iter().map(|p| p.id).max().unwrap_or(0) + 1;
        self.list.push(Place { id, name, samples: vec![fp.to_vec()] });
        Some(id)
    }

    /// Another fingerprint of a place (a few at most: the newest kept).
    pub fn add_sample(&mut self, id: u32, fp: &[Ap]) {
        if let Some(p) = self.list.iter_mut().find(|p| p.id == id) {
            if !p.samples.iter().any(|s| s == fp) {
                p.samples.push(fp.to_vec());
            }
            if p.samples.len() > 8 {
                p.samples.remove(0);
            }
        }
    }

    pub fn rename(&mut self, id: u32, name: &str) -> bool {
        let name = clean_name(name);
        if name.is_empty() || self.list.iter().any(|p| p.id != id && p.name.eq_ignore_ascii_case(&name)) {
            return false;
        }
        match self.list.iter_mut().find(|p| p.id == id) {
            Some(p) => {
                p.name = name;
                true
            }
            None => false,
        }
    }

    pub fn delete(&mut self, id: u32) {
        self.list.retain(|p| p.id != id);
    }
}

// ------------------------------------------------------------- the file ---

pub fn ap_json(ap: &Ap) -> serde_json::Value {
    serde_json::json!([bssid_text(ap.bssid), ap.dbm, ap.share])
}

pub fn ap_from(v: &serde_json::Value) -> Option<Ap> {
    Some(Ap { bssid: parse_bssid(v[0].as_str()?)?, dbm: v[1].as_i64()?.clamp(-127, 0) as i8, share: v[2].as_bool().unwrap_or(false) })
}

pub fn fp_json(fp: &[Ap]) -> serde_json::Value {
    serde_json::Value::Array(fp.iter().map(ap_json).collect())
}

pub fn fp_from(v: &serde_json::Value) -> Vec<Ap> {
    v.as_array().map(|a| a.iter().filter_map(ap_from).collect()).unwrap_or_default()
}

impl Places {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::Value::Array(
            self.list.iter().map(|p| serde_json::json!({"id": p.id, "name": p.name, "samples": p.samples.iter().map(|s| fp_json(s)).collect::<Vec<_>>()})).collect(),
        )
    }

    pub fn from_json(v: &serde_json::Value) -> Places {
        let list = v
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| {
                        Some(Place {
                            id: p["id"].as_u64()? as u32,
                            name: p["name"].as_str()?.to_owned(),
                            samples: p["samples"].as_array().map(|s| s.iter().map(fp_from).filter(|f| !f.is_empty()).collect()).unwrap_or_default(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Places { list }
    }
}

// ---------------------------------------------------------------- now ---

/// The fingerprint here and now: wpa_supplicant's last scan, and a fresh
/// one if that saw too little. Nothing where there is no Wi-Fi (the
/// simulator's stand-in shop instead).
pub fn here() -> Vec<Ap> {
    const WPA_CLI: &str = "/usr/sbin/wpa_cli";
    if !std::path::Path::new(WPA_CLI).exists() {
        return demo_here();
    }
    let cli = |args: &[&str]| {
        let out = std::process::Command::new(WPA_CLI).args(["-p", "/run/wpa_supplicant", "-i", "wlan0"]).args(args).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let first = cli(&["scan_results"]).map(|t| parse_scan(&t)).unwrap_or_default();
    if first.len() >= 3 {
        return first;
    }
    // Too little in the last scan (the radio just woke): look again.
    cli(&["scan"]);
    std::thread::sleep(std::time::Duration::from_secs(4));
    let again = cli(&["scan_results"]).map(|t| parse_scan(&t)).unwrap_or_default();
    if again.len() > first.len() {
        again
    } else {
        first
    }
}

/// Where the simulator says it is: its workshop, unless told otherwise
/// (`CATALYST_DEMO_PLACE`: shop, pit, home or none).
fn demo_here() -> Vec<Ap> {
    if crate::sys::on_handheld() {
        return Vec::new();
    }
    demo_place(&std::env::var("CATALYST_DEMO_PLACE").unwrap_or_else(|_| "shop".into()))
}

/// The simulator's made-up places, by name.
pub fn demo_place(which: &str) -> Vec<Ap> {
    let aps: &[(u64, i8)] = match which {
        "shop" => &[(0x7c_10_c9_4a_31_20, -48), (0x7c_10_c9_4a_31_24, -51), (0x3c_37_86_12_aa_01, -63), (0xa4_2b_b0_77_10_02, -70), (0x00_1a_2b_3c_4d_51, -78)],
        "pit" => &[(0x58_ef_68_10_20_30, -55), (0x58_ef_68_10_20_44, -59), (0x58_ef_68_10_21_70, -66), (0x10_20_30_40_50_61, -72), (0x58_ef_68_10_22_01, -80)],
        "home" => &[(0xd8_47_32_aa_bb_c1, -45), (0x30_23_03_11_22_33, -77), (0x9c_53_22_01_02_03, -84)],
        _ => &[],
    };
    aps.iter().map(|&(bssid, dbm)| Ap { bssid, dbm, share: true }).collect()
}
