//! Where the song is played, and how the device is opened for it.
//!
//! catalyst-audio says where sound goes in /run/catalyst-audio: `output`
//! (local, bluetooth, or "usb N"), `usb` (the USB card there: its name,
//! device and whether it has a volume of its own) and `bluetooth` (the
//! codec and rate bluealsa runs at).
//!
//! **USB headphones with their own volume** are played bit for bit: their
//! `hw` device opened at the song's own rate and depth, with no resampling,
//! mixing or software volume between (the lowest-power path there is: the
//! decoder's samples straight to the converter). If the device takes
//! neither, or something else holds it, ALSA's "default" as before.
//! **Bluetooth** is opened at the rate bluealsa's stream runs at, so
//! nothing is resampled twice. **The speaker and the jack**: "default" at
//! the codec's 48 kHz.

use std::path::{Path, PathBuf};

use crate::alsa::Format;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The codec: its speaker, or headphones on its jack.
    Local,
    Usb,
    Bluetooth,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct UsbCard {
    pub id: String,
    pub device: String,
    /// Its own playback volume (else catalyst-audio's software one, which
    /// "default" goes through).
    pub own_volume: bool,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub kind: Kind,
    pub usb: Option<UsbCard>,
    pub bluetooth_rate: Option<u32>,
    pub bluetooth_codec: Option<String>,
    /// A Bluetooth speaker, not headphones (spatial sound is for headphones).
    pub bluetooth_speaker: bool,
}

pub fn run_dir() -> PathBuf {
    PathBuf::from(std::env::var("CATALYST_AUDIO_RUN").unwrap_or_else(|_| "/run/catalyst-audio".into()))
}

fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')).map(str::trim).filter(|v| !v.is_empty())
}

pub fn parse(output: &str, usb: &str, bluetooth: &str) -> Route {
    let kind = match output.split_whitespace().next() {
        Some("usb") => Kind::Usb,
        Some("bluetooth") => Kind::Bluetooth,
        _ => Kind::Local,
    };
    let usb = field(usb, "id").map(|id| UsbCard { id: id.to_owned(), device: field(usb, "device").unwrap_or("0").to_owned(), own_volume: field(usb, "mixer").is_some(), name: field(usb, "name").unwrap_or("USB audio").to_owned() });
    let speaker = matches!(field(bluetooth, "icon"), Some("audio-card" | "audio-speakers"));
    Route { kind, usb, bluetooth_rate: field(bluetooth, "rate").and_then(|r| r.parse().ok()), bluetooth_codec: field(bluetooth, "codec").map(str::to_owned), bluetooth_speaker: speaker }
}

pub fn read(dir: &Path) -> Route {
    let get = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap_or_default();
    parse(&get("output"), &get("usb"), &get("bluetooth"))
}

/// One way to open the device.
#[derive(Clone, Debug, PartialEq)]
pub struct Attempt {
    pub device: String,
    pub rate: u32,
    pub format: Format,
    /// The device's own rate and format, or nothing (bit for bit).
    pub exact: bool,
}

impl Attempt {
    /// Played bit for bit: the device's own, at a depth that loses nothing.
    pub fn bit_perfect(&self, song_bits: u32) -> bool {
        self.exact && self.format.bits() >= song_bits
    }
}

/// The ways to try, best first. `device`: one asked for outright.
pub fn plan(route: &Route, song_rate: u32, song_bits: u32, device: Option<&str>) -> Vec<Attempt> {
    let default = |rate: u32| Attempt { device: "default".into(), rate, format: Format::S16, exact: false };
    if let Some(d) = device.filter(|d| *d != "default") {
        return vec![Attempt { device: d.to_owned(), rate: crate::RATE, format: Format::S16, exact: false }];
    }
    match (route.kind, &route.usb) {
        (Kind::Usb, Some(card)) if card.own_volume => {
            let hw = format!("hw:CARD={},DEV={}", card.id, card.device);
            // 16-bit songs at 16 bits, or padded into 32 or 24 for a device
            // that takes only those; 24-bit at 24 (three bytes) or 32.
            let formats: &[Format] = match song_bits {
                0..=16 => &[Format::S16, Format::S32, Format::S24In3],
                17..=24 => &[Format::S24In3, Format::S32],
                _ => &[Format::S32],
            };
            let mut ways: Vec<Attempt> = formats.iter().map(|&format| Attempt { device: hw.clone(), rate: song_rate, format, exact: true }).collect();
            ways.push(default(crate::RATE));
            ways
        }
        (Kind::Bluetooth, _) => vec![default(route.bluetooth_rate.filter(|r| (8_000..=96_000).contains(r)).unwrap_or(crate::RATE))],
        _ => vec![default(crate::RATE)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USB: &str = "card=1\nid=Headset\ndevice=0\nusbid=0d8c:0014\nname=USB Audio Device\nmixer=Headphone,0\nswitch=yes\n";

    #[test]
    fn usb_headphones_with_their_own_volume_are_opened_at_the_songs_own_rate() {
        let r = parse("usb 1\n", USB, "");
        assert_eq!(r.kind, Kind::Usb);
        let ways = plan(&r, 44_100, 16, None);
        assert_eq!(ways[0], Attempt { device: "hw:CARD=Headset,DEV=0".into(), rate: 44_100, format: Format::S16, exact: true });
        assert!(ways[0].bit_perfect(16));
        assert_eq!(ways.last().unwrap().device, "default");
        assert!(!ways.last().unwrap().bit_perfect(16));
        // 24-bit: never at 16.
        let deep = plan(&r, 96_000, 24, None);
        assert!(deep.iter().filter(|a| a.exact).all(|a| a.format.bits() >= 24 && a.rate == 96_000));
    }

    #[test]
    fn software_volume_bluetooth_and_the_speaker_go_through_default() {
        let soft = parse("usb 1\n", "card=1\nid=Headset\ndevice=0\nmixer=\n", "");
        assert_eq!(plan(&soft, 44_100, 16, None), vec![Attempt { device: "default".into(), rate: 48_000, format: Format::S16, exact: false }]);
        let bt = parse("bluetooth\n", "", "codec=AAC\nrate=44100\n");
        assert_eq!((bt.bluetooth_codec.as_deref(), plan(&bt, 48_000, 16, None)[0].rate), (Some("AAC"), 44_100));
        let local = parse("local\n", "", "");
        assert_eq!(plan(&local, 44_100, 16, None)[0].rate, 48_000);
        assert_eq!(plan(&local, 44_100, 16, Some("plughw:0"))[0].device, "plughw:0");
        assert_eq!(parse("", "", "").kind, Kind::Local);
    }
}
