//! The town a photo was taken in, from its Wi-Fi: opt-in, since it sends the
//! access points' addresses off the handheld.
//!
//! **Where.** BeaconDB (beacondb.net), a public-domain Wi-Fi database run as
//! Mozilla Location Service's successor, through its geolocate API
//! (`POST https://api.beacondb.net/v1/geolocate`), which keeps Ichnaea's
//! request and reply. Its site asks clients to say who they are in the
//! User-Agent; this one says `CatalystLinux-Photos/1.4` and nothing about
//! the team or the handheld. Its privacy notice says the addresses asked
//! about and the place answered are not recorded (its web server's log of
//! the request, the time and the User-Agent goes after 28 days).
//!
//! **What is sent.** Only the access points' addresses and signal, two at
//! least (Ichnaea's own floor, for privacy), never a hidden network's or one
//! named `…_nomap` (Ichnaea: "must not be used"), nor a robot's radio or a
//! phone's hotspot (places.rs never keeps those). The fallbacks are turned
//! off (`considerIp: false`, no GeoIP, no cell area): a position from the
//! Wi-Fi, or none.
//!
//! **The town.** The position becomes a town on the handheld, offline
//! (gazetteer.rs). A photo of a place already looked up, alike enough to be
//! the same place, takes that town without asking again; and a photo at a
//! named place or at an event is never looked up, since its place line
//! needs no town.

use std::sync::Mutex;

use serde_json::{json, Value};

use super::caption::{backoff, Conditions, Did};
use super::net;
use super::places::{self, Ap};
use super::store::{Job, Ledger, Spot};

pub const BEACONDB: &str = "https://api.beacondb.net";
const AGENT: &str = "CatalystLinux-Photos/1.4";
/// Look-ups in one round at most.
const ROUND: usize = 5;
const TRIES: u32 = 6;

/// The geolocate request for a fingerprint, if it has enough that may be sent.
pub fn request_body(fp: &[Ap]) -> Option<Value> {
    let aps: Vec<Value> = fp.iter().filter(|a| a.share).map(|a| json!({"macAddress": places::bssid_text(a.bssid), "signalStrength": a.dbm})).collect();
    (aps.len() >= 2).then(|| json!({"considerIp": false, "fallbacks": {"lacf": false, "ipf": false}, "wifiAccessPoints": aps}))
}

/// The answer: a position, nothing known (404), or trouble.
pub fn parse(code: u16, body: &str) -> Result<Option<Spot>, String> {
    match code {
        200 => {
            let v: Value = serde_json::from_str(body).map_err(|_| "beacondb's answer could not be read".to_owned())?;
            let (lat, lon) = (v["location"]["lat"].as_f64(), v["location"]["lng"].as_f64());
            match (lat, lon) {
                // A guess from the IP address is not where the photo was.
                _ if v["fallback"].is_string() => Ok(None),
                (Some(lat), Some(lon)) => Ok(Some(Spot { lat, lon, accuracy: v["accuracy"].as_f64().unwrap_or(0.0) })),
                _ => Err("beacondb's answer had no position".into()),
            }
        }
        404 => Ok(None),
        c => Err(format!("beacondb answered {c}")),
    }
}

/// A town for a position, from the gazetteer.
pub fn town_at(spot: &Spot) -> Option<String> {
    super::gazetteer::nearest(spot.lat, spot.lon).map(|f| f.words)
}

fn lock(m: &Mutex<Ledger>) -> std::sync::MutexGuard<'_, Ledger> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn give(l: &mut Ledger, name: &str, spot: Option<Spot>, town: Option<String>) {
    let path = l.photo_path(name);
    if let Some(r) = l.records.get_mut(name) {
        r.spot = spot;
        r.town = if town.is_some() { Job::Done } else { Job::Skipped("not known to beacondb".into()) };
    }
    if let Some(t) = town {
        let mut about = super::meta::read(&path);
        about.town = t;
        super::meta::write(&path, &about).ok();
    }
}

/// One round of town look-ups: `base` is BeaconDB's address (the tests'
/// stand-in's in tests); `demo` answers here instead.
pub fn step(ledger: &Mutex<Ledger>, base: &str, demo: bool, now: i64, cond: Conditions) -> Did {
    let due: Vec<(String, Vec<Ap>)> = {
        let l = lock(ledger);
        if !l.prefs.town {
            return Did::Waits("the town lookup is off");
        }
        if let Some(why) = cond.waits() {
            return Did::Waits(why);
        }
        l.records.iter().filter(|(_, r)| matches!(r.town, Job::Waiting { next, .. } if next <= now)).take(ROUND).map(|(n, r)| (n.clone(), r.fp.clone())).collect()
    };
    if due.is_empty() {
        let l = lock(ledger);
        let next = l.records.values().filter_map(|r| if let Job::Waiting { next, .. } = r.town { Some(next) } else { None }).min();
        return Did::Nothing(next);
    }
    let mut took = 0;
    for (name, fp) in due {
        // The same place, already looked up: its town, no request.
        let known = {
            let l = lock(ledger);
            l.records.iter().find(|(n, r)| **n != name && r.town == Job::Done && r.spot.is_some() && places::same_place(&r.fp, &fp)).map(|(n, r)| (r.spot, super::meta::read(&l.photo_path(n)).town))
        };
        if let Some((spot, town)) = known.filter(|(_, t)| !t.is_empty()) {
            let mut l = lock(ledger);
            give(&mut l, &name, spot, Some(town));
            l.save();
            took += 1;
            continue;
        }
        let Some(body) = request_body(&fp) else {
            let mut l = lock(ledger);
            if let Some(r) = l.records.get_mut(&name) {
                r.town = Job::Skipped("too few networks to look up".into());
            }
            l.save();
            continue;
        };
        let reply = if demo {
            // The simulator's: a spot by Rochester, no request made.
            Ok(net::Reply { code: 200, body: json!({"location": {"lat": 43.0848, "lng": -77.6744}, "accuracy": 40.0}).to_string() })
        } else {
            net::call("POST", &format!("{base}/v1/geolocate"), "content-type: application/json\n", Some(body.to_string().as_bytes()), AGENT)
        };
        let mut l = lock(ledger);
        match reply.map_err(|e| (0u16, e)).and_then(|r| parse(r.code, &r.body).map_err(|e| (r.code, e))) {
            Ok(spot) => {
                let town = spot.as_ref().and_then(town_at);
                give(&mut l, &name, spot, town);
                took += 1;
            }
            // A request it could not take is not asked again.
            Err((400, why)) => {
                if let Some(r) = l.records.get_mut(&name) {
                    r.town = Job::Failed(why);
                }
            }
            Err((_, why)) => {
                if let Some(r) = l.records.get_mut(&name) {
                    let tries = if let Job::Waiting { tries, .. } = r.town { tries + 1 } else { 1 };
                    r.town = if tries >= TRIES { Job::Failed(why.clone()) } else { Job::Waiting { tries, next: now + backoff(tries) } };
                }
                l.save();
                return Did::Failed(why);
            }
        }
        l.save();
    }
    Did::Took(took)
}
