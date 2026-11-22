//! AAP, the protocol an iPhone speaks with AirPods (and Beats) beside the
//! audio: an L2CAP channel on PSM 0x1001 of the Bluetooth link already
//! there. Apple has not published it; this follows what the LibrePods
//! project (github.com/kavishdevar/librepods: its `AAP Definitions.md`,
//! Android `AACPManager.kt` and Linux `airpods_packets.h`) found, with the
//! packets it gives as tests.
//!
//! A session: the handshake (nothing is answered before it), the features
//! asked for, notifications asked for; then the AirPods say their battery,
//! which buds are in the ears, the noise control mode and who they are,
//! as each changes. Head tracking is asked for (and stopped) by a packet
//! each way, and streams the buds' orientation while it runs.
//!
//! Every packet after the handshake starts `04 00 04 00`, then an opcode
//! (a little-endian u16).

#[cfg(target_os = "linux")]
pub mod l2cap;

pub const PSM: u16 = 0x1001;

pub const HANDSHAKE: [u8; 16] = [0x00, 0x00, 0x04, 0x00, 0x01, 0x00, 0x02, 0x00, 0, 0, 0, 0, 0, 0, 0, 0];
/// Conversational awareness and adaptive transparency, as LibrePods asks
/// for them (`SET_SPECIFIC_FEATURES`).
pub const SET_FEATURES: [u8; 14] = [0x04, 0x00, 0x04, 0x00, 0x4D, 0x00, 0xD7, 0x00, 0, 0, 0, 0, 0, 0];
pub const REQUEST_NOTIFICATIONS: [u8; 10] = [0x04, 0x00, 0x04, 0x00, 0x0F, 0x00, 0xFF, 0xFF, 0xFF, 0xFF];

const HEADER: [u8; 4] = [0x04, 0x00, 0x04, 0x00];

mod op {
    pub const BATTERY: u8 = 0x04;
    pub const EAR: u8 = 0x06;
    pub const CONTROL: u8 = 0x09;
    pub const HEAD: u8 = 0x17;
    pub const INFO: u8 = 0x1D;
    pub const FEATURES_ACK: u8 = 0x2B;
    pub const CONVERSATION: u8 = 0x4B;
}

/// Control command identifiers (opcode 0x09).
mod control {
    pub const LISTENING_MODE: u8 = 0x0D;
}

/// Head tracking's start and stop, and the older firmware's spellings of
/// them (LibrePods' "alternate" packets), tried when the first gets no data.
pub fn head_tracking(start: bool, alternate: bool) -> Vec<u8> {
    let body: &[u8] = match (start, alternate) {
        (true, false) => &[0x00, 0x00, 0x10, 0x00, 0x10, 0x00, 0x08, 0xA1, 0x02, 0x42, 0x0B, 0x08, 0x0E, 0x10, 0x02, 0x1A, 0x05, 0x01, 0x40, 0x9C, 0x00, 0x00],
        (true, true) => &[0x00, 0x00, 0x10, 0x00, 0x0F, 0x00, 0x08, 0x73, 0x42, 0x0B, 0x08, 0x10, 0x10, 0x02, 0x1A, 0x05, 0x01, 0x40, 0x9C, 0x00, 0x00],
        (false, false) => &[0x00, 0x00, 0x10, 0x00, 0x11, 0x00, 0x08, 0x7E, 0x10, 0x02, 0x42, 0x0B, 0x08, 0x4E, 0x10, 0x02, 0x1A, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00],
        (false, true) => &[0x00, 0x00, 0x10, 0x00, 0x0F, 0x00, 0x08, 0x75, 0x42, 0x0B, 0x08, 0x10, 0x10, 0x02, 0x1A, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00],
    };
    let mut p = HEADER.to_vec();
    p.extend_from_slice(&[op::HEAD, 0x00]);
    p.extend_from_slice(body);
    p
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseMode {
    Off,
    Cancelling,
    Transparency,
    Adaptive,
}

impl NoiseMode {
    pub const ALL: [NoiseMode; 4] = [NoiseMode::Off, NoiseMode::Cancelling, NoiseMode::Transparency, NoiseMode::Adaptive];

    fn byte(self) -> u8 {
        match self {
            NoiseMode::Off => 1,
            NoiseMode::Cancelling => 2,
            NoiseMode::Transparency => 3,
            NoiseMode::Adaptive => 4,
        }
    }

    fn from_byte(b: u8) -> Option<NoiseMode> {
        NoiseMode::ALL.into_iter().find(|m| m.byte() == b)
    }

    pub fn word(self) -> &'static str {
        match self {
            NoiseMode::Off => "off",
            NoiseMode::Cancelling => "noise cancelling",
            NoiseMode::Transparency => "transparency",
            NoiseMode::Adaptive => "adaptive",
        }
    }
}

/// A control command: `04 00 04 00 09 00 <id> <four bytes of value>`.
pub fn control_command(id: u8, value: [u8; 4]) -> Vec<u8> {
    let mut p = HEADER.to_vec();
    p.extend_from_slice(&[op::CONTROL, 0x00, id]);
    p.extend_from_slice(&value);
    p
}

/// Noise control set: the AirPods answer with the mode they took.
pub fn noise_control(mode: NoiseMode) -> Vec<u8> {
    control_command(control::LISTENING_MODE, [mode.byte(), 0, 0, 0])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// AirPods Max: one battery.
    Headphones,
    Right,
    Left,
    Case,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Charge {
    Unknown,
    Charging,
    Discharging,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub part: Part,
    /// Percent.
    pub level: u8,
    pub charge: Charge,
}

/// Where a bud is: the primary's and the secondary's (the primary is the
/// one the AirPods talk through; it changes as buds come out).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ear {
    In,
    Out,
    InCase,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Info {
    pub name: String,
    /// Apple's model number ("A2698").
    pub model: String,
    pub manufacturer: String,
    pub serial: String,
    pub firmware: String,
}

/// The buds' orientation while head tracking runs, as they send it: three
/// 16-bit values (LibrePods' `o1`, `o2`, `o3`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeadRaw(pub i16, pub i16, pub i16);

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    HandshakeAck,
    FeaturesAck,
    Battery(Vec<Cell>),
    Ears(Ear, Ear),
    Noise(NoiseMode),
    Control { id: u8, value: [u8; 4] },
    Info(Info),
    Head(HeadRaw),
    /// Conversational awareness: someone is speaking (1 to 9; 1 to 3, the
    /// sound turned down for it).
    Conversation(u8),
    Other(u8),
}

/// A packet from the AirPods.
pub fn parse(p: &[u8]) -> Option<Message> {
    if p.starts_with(&[0x01, 0x00, 0x04, 0x00]) {
        return Some(Message::HandshakeAck);
    }
    if !p.starts_with(&HEADER) || p.len() < 6 {
        return None;
    }
    let body = &p[6..];
    Some(match p[4] {
        op::BATTERY => {
            let count = *body.first()? as usize;
            let mut cells = Vec::new();
            for i in 0..count {
                let Some(c) = body.get(1 + i * 5..1 + i * 5 + 5) else { break };
                let part = match c[0] {
                    0x01 => Part::Headphones,
                    0x02 => Part::Right,
                    0x04 => Part::Left,
                    0x08 => Part::Case,
                    _ => continue,
                };
                let charge = match c[3] {
                    0x01 => Charge::Charging,
                    0x02 => Charge::Discharging,
                    // Disconnected (the case shut, a bud out of reach): not there.
                    0x04 => continue,
                    _ => Charge::Unknown,
                };
                cells.push(Cell { part, level: c[2].min(100), charge });
            }
            Message::Battery(cells)
        }
        op::EAR => {
            let ear = |b: u8| match b {
                0x00 => Ear::In,
                0x02 => Ear::InCase,
                _ => Ear::Out,
            };
            Message::Ears(ear(*body.first()?), ear(*body.get(1).unwrap_or(&0x01)))
        }
        op::CONTROL => {
            let id = *body.first()?;
            let mut value = [0u8; 4];
            for (v, b) in value.iter_mut().zip(body.iter().skip(1)) {
                *v = *b;
            }
            match (id, NoiseMode::from_byte(value[0])) {
                (control::LISTENING_MODE, Some(mode)) => Message::Noise(mode),
                _ => Message::Control { id, value },
            }
        }
        op::INFO => Message::Info(parse_info(p)),
        // The start and stop are echoed short; data is long.
        op::HEAD if p.len() >= 55 => {
            let v = |i: usize| i16::from_le_bytes([p[i], p[i + 1]]);
            Message::Head(HeadRaw(v(43), v(45), v(47)))
        }
        op::FEATURES_ACK => Message::FeaturesAck,
        op::CONVERSATION if body.len() >= 4 => Message::Conversation(body[3]),
        other => Message::Other(other),
    })
}

/// The information packet: five bytes of header, six more, then strings,
/// each ended by a zero: name, model, manufacturer, serial, firmware, ...
fn parse_info(p: &[u8]) -> Info {
    let strings: Vec<String> = p.get(11..).unwrap_or_default().split(|&b| b == 0).map(|s| String::from_utf8_lossy(s).into_owned()).collect();
    let at = |i: usize| strings.get(i).cloned().unwrap_or_default();
    Info { name: at(0), model: at(1), manufacturer: at(2), serial: at(3), firmware: at(4) }
}

/// Whether a device is Apple's, by the modalias BlueZ shows for it (its
/// Device ID record: `bluetooth:v004Cp2014d0E10`, vendor 0x004C).
pub fn is_apple(modalias: &str) -> bool {
    let m = modalias.trim().to_ascii_lowercase();
    m.starts_with("bluetooth:v004c") || m.starts_with("usb:v05ac")
}

/// AirPods with adaptive transparency: AirPods Pro (2nd and 3rd
/// generation) and AirPods 4 with noise cancelling, by model number.
pub fn has_adaptive(model: &str) -> bool {
    const MODELS: [&str; 14] = ["A2698", "A2699", "A2931", "A3047", "A3048", "A3049", "A3055", "A3056", "A3057", "A3058", "A3059", "A3063", "A3064", "A3065"];
    MODELS.contains(&model.trim())
}

/// AirPods Max, which have no case and one battery.
pub fn is_max(model: &str) -> bool {
    ["A2096", "A3184"].contains(&model.trim())
}

/// The buds' orientation as a yaw and pitch, in degrees, as LibrePods reads
/// it: the first samples taken as straight ahead, then the second and
/// third values' difference is the yaw and their sum the pitch, a full
/// 32000 counting as 180°.
pub struct Orientation {
    samples: Vec<(i32, i32, i32)>,
    neutral: Option<(i32, i32)>,
}

impl Default for Orientation {
    fn default() -> Self {
        Self::new()
    }
}

impl Orientation {
    const CALIBRATE: usize = 10;
    const SCALE: f32 = 180.0 / 32000.0;

    pub fn new() -> Orientation {
        Orientation { samples: Vec::new(), neutral: None }
    }

    /// The yaw (+ right) and pitch (+ up), once the first samples are in.
    pub fn update(&mut self, raw: HeadRaw) -> Option<(f32, f32)> {
        let (o2, o3) = (raw.1 as i32, raw.2 as i32);
        let Some((n2, n3)) = self.neutral else {
            self.samples.push((raw.0 as i32, o2, o3));
            if self.samples.len() >= Self::CALIBRATE {
                let n = self.samples.len() as i32;
                let (s2, s3) = self.samples.iter().fold((0, 0), |(a, b), s| (a + s.1, b + s.2));
                self.neutral = Some((s2 / n, s3 / n));
            }
            return None;
        };
        let (d2, d3) = ((o2 - n2) as f32, (o3 - n3) as f32);
        let yaw = (d2 - d3) / 2.0 * Self::SCALE;
        let pitch = (d2 + d3) / 2.0 * Self::SCALE;
        Some((yaw, pitch))
    }

    pub fn reset(&mut self) {
        self.samples.clear();
        self.neutral = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
    }

    #[test]
    fn the_session_packets_are_librepods_own() {
        assert_eq!(HANDSHAKE.to_vec(), hex("00 00 04 00 01 00 02 00 00 00 00 00 00 00 00 00"));
        assert_eq!(SET_FEATURES.to_vec(), hex("04 00 04 00 4d 00 d7 00 00 00 00 00 00 00"));
        assert_eq!(REQUEST_NOTIFICATIONS.to_vec(), hex("04 00 04 00 0F 00 FF FF FF FF"));
        // AAP Definitions.md, "Head Tracking".
        assert_eq!(head_tracking(true, false), hex("04 00 04 00 17 00 00 00 10 00 10 00 08 A1 02 42 0B 08 0E 10 02 1A 05 01 40 9C 00 00"));
        assert_eq!(head_tracking(false, false), hex("04 00 04 00 17 00 00 00 10 00 11 00 08 7E 10 02 42 0B 08 4E 10 02 1A 05 01 00 00 00 00"));
        assert_eq!(head_tracking(true, true).len(), 27);
        // "Noise Control": 04 00 04 00 09 00 0D [mode] 00 00 00.
        assert_eq!(noise_control(NoiseMode::Cancelling), hex("04 00 04 00 09 00 0D 02 00 00 00"));
        assert_eq!(noise_control(NoiseMode::Adaptive), hex("04 00 04 00 09 00 0D 04 00 00 00"));
        assert_eq!(noise_control(NoiseMode::Off), hex("04 00 04 00 09 00 0D 01 00 00 00"));
    }

    #[test]
    fn the_battery_capture_reads_bud_by_bud() {
        // AAP Definitions.md's example. By its component codes (right 02,
        // left 04, case 08, as LibrePods' battery.hpp has them): right 100 %
        // discharging, left 99 % charging, case 17 % discharging.
        let p = hex("04 00 04 00 04 00 03 02 01 64 02 01 04 01 63 01 01 08 01 11 02 01");
        assert_eq!(
            parse(&p),
            Some(Message::Battery(vec![
                Cell { part: Part::Right, level: 100, charge: Charge::Discharging },
                Cell { part: Part::Left, level: 99, charge: Charge::Charging },
                Cell { part: Part::Case, level: 17, charge: Charge::Discharging },
            ]))
        );
        // The case shut: its cell says disconnected, and is left out.
        let shut = hex("04 00 04 00 04 00 02 02 01 50 02 01 08 01 00 04 01");
        assert_eq!(parse(&shut), Some(Message::Battery(vec![Cell { part: Part::Right, level: 80, charge: Charge::Discharging }])));
        // Cut short: no panic, what is there.
        assert_eq!(parse(&hex("04 00 04 00 04 00 03 02 01")), Some(Message::Battery(vec![])));
    }

    #[test]
    fn ears_noise_and_acks_are_read() {
        assert_eq!(parse(&hex("04 00 04 00 06 00 00 01")), Some(Message::Ears(Ear::In, Ear::Out)));
        assert_eq!(parse(&hex("04 00 04 00 06 00 02 00")), Some(Message::Ears(Ear::InCase, Ear::In)));
        assert_eq!(parse(&hex("04 00 04 00 09 00 0D 03 00 00 00")), Some(Message::Noise(NoiseMode::Transparency)));
        // Conversational awareness's state, another control command.
        assert_eq!(parse(&hex("04 00 04 00 09 00 28 01 00 00 00")), Some(Message::Control { id: 0x28, value: [1, 0, 0, 0] }));
        assert_eq!(parse(&hex("04 00 04 00 4B 00 02 00 01 03")), Some(Message::Conversation(3)));
        assert_eq!(parse(&hex("01 00 04 00 00 00")), Some(Message::HandshakeAck));
        assert_eq!(parse(&hex("04 00 04 00 2b 00 00")), Some(Message::FeaturesAck));
        assert_eq!(parse(&hex("04 00 04 00 ee 00")), Some(Message::Other(0xEE)));
        assert_eq!(parse(&hex("05 00")), None);
    }

    #[test]
    fn who_they_are_and_where_the_head_points() {
        let mut p = hex("04 00 04 00 1D 00 01 02 03 04 05");
        p.extend_from_slice(b"Thomas's AirPods Pro\0A2698\0Apple Inc.\0H3KXXXXXX\06F21\0");
        let Some(Message::Info(info)) = parse(&p) else { panic!() };
        assert_eq!((info.name.as_str(), info.model.as_str(), info.firmware.as_str()), ("Thomas's AirPods Pro", "A2698", "6F21"));
        assert!(has_adaptive(&info.model) && !has_adaptive("A2084"));
        // A head tracking frame: 70-odd bytes, the orientation at 43, 45 and 47.
        let mut head = vec![0u8; 80];
        head[..6].copy_from_slice(&hex("04 00 04 00 17 00"));
        head[43..45].copy_from_slice(&19000i16.to_le_bytes());
        head[45..47].copy_from_slice(&(-120i16).to_le_bytes());
        head[47..49].copy_from_slice(&300i16.to_le_bytes());
        assert_eq!(parse(&head), Some(Message::Head(HeadRaw(19000, -120, 300))));
        // The start's echo is short: not a frame.
        assert_eq!(parse(&head_tracking(true, false)), Some(Message::Other(0x17)));
        assert!(is_apple("bluetooth:v004Cp2014d0E10") && !is_apple("bluetooth:v0075p0100d0200"));
    }

    #[test]
    fn the_orientation_is_calibrated_then_read_as_yaw_and_pitch() {
        let mut o = Orientation::new();
        for _ in 0..9 {
            assert_eq!(o.update(HeadRaw(19000, 100, 100)), None);
        }
        assert_eq!(o.update(HeadRaw(19000, 100, 100)), None);
        assert_eq!(o.update(HeadRaw(19000, 100, 100)), Some((0.0, 0.0)));
        // o2 up and o3 down by the same: all yaw.
        let (yaw, pitch) = o.update(HeadRaw(19000, 100 + 8000, 100 - 8000)).unwrap();
        assert!((yaw - 45.0).abs() < 0.01 && pitch.abs() < 0.01);
        let (yaw, pitch) = o.update(HeadRaw(19000, 100 + 1600, 100 + 1600)).unwrap();
        assert!(yaw.abs() < 0.01 && (pitch - 9.0).abs() < 0.01);
    }
}
