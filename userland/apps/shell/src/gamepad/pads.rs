//! The controllers plugged in: found as their nodes appear in /dev/input
//! (and let go as they leave), their events read as they come. In the
//! simulator, and wherever evdev is missing, one simulated controller
//! stands in, sitting with a little drift and shake, or swept round its
//! edges while a check asks for that.

use super::evdev::*;

pub struct Pads {
    pub list: Vec<Pad>,
    #[cfg(target_os = "linux")]
    devices: Vec<linux::Device>,
    #[cfg(target_os = "linux")]
    watch: Option<std::fs::File>,
    /// Nodes looked at that are not controllers (a keyboard), not asked again.
    #[cfg(target_os = "linux")]
    others: Vec<String>,
    since_scan: f32,
    sim: Option<Sim>,
}

/// How often the nodes are looked over without a sign from the watch.
const SCAN_EVERY: f32 = 2.0;

impl Pads {
    /// The real controllers on a Linux machine; the simulated one elsewhere.
    pub fn open() -> Pads {
        if cfg!(target_os = "linux") && std::env::var_os("CATALYST_SIM_GAMEPAD").is_none() {
            let mut pads = Pads::empty();
            #[cfg(target_os = "linux")]
            {
                pads.watch = linux::watch();
            }
            pads.scan();
            pads
        } else {
            Pads::simulated()
        }
    }

    fn empty() -> Pads {
        Pads {
            list: Vec::new(),
            #[cfg(target_os = "linux")]
            devices: Vec::new(),
            #[cfg(target_os = "linux")]
            watch: None,
            #[cfg(target_os = "linux")]
            others: Vec::new(),
            since_scan: 0.0,
            sim: None,
        }
    }

    pub fn simulated() -> Pads {
        let mut pads = Pads::empty();
        pads.list.push(Sim::pad());
        pads.sim = Some(Sim::default());
        pads
    }

    /// With no controller plugged in (the simulator's gallery).
    #[cfg(not(target_arch = "aarch64"))]
    pub fn none() -> Pads {
        Pads::empty()
    }

    /// The simulated controller is swept round its edges (a check's sweep).
    pub fn sweep(&mut self, on: bool) {
        if let Some(sim) = &mut self.sim {
            sim.sweeping = on;
        }
    }

    /// The simulated controller held as given (the gallery's pictures).
    #[cfg(not(target_arch = "aarch64"))]
    pub fn hold(&mut self, sticks: [(f32, f32); 2], triggers: [f32; 2], down: &[u16], hat: (i32, i32)) {
        let at = match &mut self.sim {
            Some(sim) => {
                sim.clock_us += 8000;
                sim.clock_us
            }
            None => 0,
        };
        let Some(pad) = self.list.first_mut() else { return };
        let raw = |v: f32| (v.clamp(-1.0, 1.0) * 32767.0).round() as i32;
        let mut events = Vec::new();
        for (code, v) in [(ABS_X, sticks[0].0), (ABS_Y, sticks[0].1), (ABS_RX, sticks[1].0), (ABS_RY, sticks[1].1)] {
            events.push(Event::new(0, EV_ABS, code, raw(v)));
        }
        events.push(Event::new(0, EV_ABS, ABS_Z, (triggers[0] * 1023.0) as i32));
        events.push(Event::new(0, EV_ABS, ABS_RZ, (triggers[1] * 1023.0) as i32));
        events.push(Event::new(0, EV_ABS, ABS_HAT0X, hat.0));
        events.push(Event::new(0, EV_ABS, ABS_HAT0Y, hat.1));
        let codes: Vec<u16> = pad.buttons.iter().map(|(c, _)| *c).collect();
        for c in codes {
            events.push(Event::new(0, EV_KEY, c, down.contains(&c) as i32));
        }
        // Two reports 4 ms apart, so it has a rate as a real one would.
        events.push(Event::new(at - 4000, EV_SYN, SYN_REPORT, 0));
        events.push(Event::new(at, EV_SYN, SYN_REPORT, 0));
        pad.feed(&events);
        if let Some(sim) = &mut self.sim {
            sim.held = true;
        }
    }

    /// Look the nodes over: new controllers opened, gone ones let go.
    /// True if the list changed.
    pub fn scan(&mut self) -> bool {
        self.since_scan = 0.0;
        #[cfg(target_os = "linux")]
        {
            let nodes: Vec<String> = std::fs::read_dir("/dev/input").into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("event")).collect();
            let before = self.list.len();
            let mut changed = false;
            let mut i = 0;
            while i < self.list.len() {
                if nodes.contains(&self.list[i].node) {
                    i += 1;
                } else {
                    self.list.remove(i);
                    self.devices.remove(i);
                    changed = true;
                }
            }
            self.others.retain(|n| nodes.contains(n));
            for node in nodes {
                if self.list.iter().any(|p| p.node == node) || self.others.contains(&node) {
                    continue;
                }
                match linux::open(&node) {
                    Some((device, pad)) => {
                        self.devices.push(device);
                        self.list.push(pad);
                    }
                    None => self.others.push(node),
                }
            }
            return changed || self.list.len() != before;
        }
        #[allow(unreachable_code)]
        false
    }

    /// Read what has come. True if any report arrived or the list changed.
    pub fn poll(&mut self, dt: f32) -> bool {
        if let Some(sim) = &mut self.sim {
            return match self.list.first_mut() {
                Some(pad) => sim.run(pad, dt),
                None => false,
            };
        }
        #[allow(unused_mut)]
        let mut any = false;
        self.since_scan += dt;
        #[cfg(target_os = "linux")]
        {
            use std::io::Read;
            let mut woke = false;
            if let Some(w) = &mut self.watch {
                let mut buf = [0u8; 512];
                while let Ok(n) = w.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    woke = true;
                }
            }
            if woke || self.since_scan >= SCAN_EVERY {
                any |= self.scan();
            }
            let mut gone = Vec::new();
            for (i, (device, pad)) in self.devices.iter_mut().zip(self.list.iter_mut()).enumerate() {
                match device.read() {
                    Ok(events) if !events.is_empty() => {
                        let lost = events.iter().any(|e| e.kind == EV_SYN && e.code == SYN_DROPPED);
                        any |= pad.feed(&events);
                        if lost && !pad.dropped {
                            device.resync(pad);
                        }
                    }
                    Ok(_) => {}
                    Err(()) => gone.push(i),
                }
            }
            for i in gone.into_iter().rev() {
                self.list.remove(i);
                self.devices.remove(i);
                any = true;
            }
        }
        #[cfg(not(target_os = "linux"))]
        if self.since_scan >= SCAN_EVERY {
            self.since_scan = 0.0;
        }
        any
    }

    /// Rumble a controller for `ms`: false where it cannot.
    pub fn rumble(&mut self, i: usize, ms: u16) -> bool {
        if self.sim.is_some() {
            return self.list.get(i).is_some_and(|p| p.rumble);
        }
        #[cfg(target_os = "linux")]
        if let Some(device) = self.devices.get_mut(i) {
            return device.rumble(ms);
        }
        let _ = (i, ms);
        false
    }
}

/// The simulated controller: an Xbox One S pad on USB (xpad's ranges).
#[derive(Default)]
pub struct Sim {
    clock_us: i64,
    owed: f32,
    seed: u32,
    pub sweeping: bool,
    angle: f32,
    /// Posed for a picture: it sends nothing of its own.
    held: bool,
}

/// It reports every 4 ms (250 a second), as a wired Xbox pad does.
const EVERY_S: f32 = 0.004;

impl Sim {
    pub fn pad() -> Pad {
        let stick = AbsInfo { value: 0, min: -32768, max: 32767, fuzz: 16, flat: 128 };
        let trigger = AbsInfo { value: 0, min: 0, max: 1023, fuzz: 0, flat: 0 };
        let hat = AbsInfo { value: 0, min: -1, max: 1, fuzz: 0, flat: 0 };
        let axes = vec![(ABS_X, stick), (ABS_Y, stick), (ABS_Z, trigger), (ABS_RX, stick), (ABS_RY, stick), (ABS_RZ, trigger), (ABS_HAT0X, hat), (ABS_HAT0Y, hat)];
        let keys = vec![BTN_SOUTH, BTN_EAST, BTN_NORTH, BTN_WEST, BTN_TL, BTN_TR, BTN_SELECT, BTN_START, BTN_MODE, BTN_THUMBL, BTN_THUMBR];
        Pad::new("event9", "Microsoft X-Box One S pad", Ident { bus: 3, vendor: 0x045e, product: 0x02ea, version: 0x0301 }, "3039373132", axes, keys, true)
    }

    fn noise(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
    }

    /// The sticks now: at rest, the left drifting a little up and right
    /// and shaking; swept, round their edges (the right one worn short).
    fn sticks(&mut self) -> [(f32, f32); 2] {
        if self.sweeping {
            let (a, b) = (self.angle, self.angle * 0.83 + 1.0);
            [(0.985 * a.cos(), 0.985 * a.sin()), (0.9 * b.cos(), 0.9 * b.sin())]
        } else {
            [(0.046 + 0.006 * self.noise(), -0.031 + 0.006 * self.noise()), (0.008 + 0.002 * self.noise(), 0.003 + 0.002 * self.noise())]
        }
    }

    fn run(&mut self, pad: &mut Pad, dt: f32) -> bool {
        if self.held {
            return false;
        }
        self.owed += dt;
        let mut any = false;
        let mut n = 0;
        while self.owed >= EVERY_S && n < 128 {
            self.owed -= EVERY_S;
            n += 1;
            self.clock_us += (EVERY_S * 1e6) as i64;
            if self.sweeping {
                self.angle += std::f32::consts::TAU * 1.3 * EVERY_S;
            }
            let s = self.sticks();
            let raw = |v: f32| (v.clamp(-1.0, 1.0) * 32767.0).round() as i32;
            let at = self.clock_us;
            let events = [
                Event::new(at, EV_ABS, ABS_X, raw(s[0].0)),
                Event::new(at, EV_ABS, ABS_Y, raw(s[0].1)),
                Event::new(at, EV_ABS, ABS_RX, raw(s[1].0)),
                Event::new(at, EV_ABS, ABS_RY, raw(s[1].1)),
                Event::new(at, EV_SYN, SYN_REPORT, 0),
            ];
            any |= pad.feed(&events);
        }
        self.owed = self.owed.min(EVERY_S);
        any
    }
}
