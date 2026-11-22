//! What is done to the sound after the songs are mixed, at the device's
//! rate: the equalizer, spatial sound (headphones only, turned by the
//! AirPods' head tracking when they send it), and a fade. Each is built
//! only while it is on; with none on, the player does not come here.

use std::time::Instant;

use audiofx::crossfade::Fade;
use audiofx::eq::Equalizer;
use audiofx::head::Recenter;
use audiofx::settings::Fx;
use audiofx::spatial::Spatial;

/// The stage comes round to where the head has turned over this long.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const RECENTER_SECONDS: f32 = 8.0;
/// No pose for this long: tracking has stopped, and the stage goes back
/// to straight ahead.
const POSES_STOPPED: f32 = 1.5;

pub struct Chain {
    rate: u32,
    eq: Equalizer,
    spatial: Option<Spatial>,
    head: Option<Head>,
    fade: Option<Fade>,
    /// The level a finished fade left (0 to 1).
    level: f32,
    faded: bool,
}

impl Chain {
    pub fn new(rate: u32) -> Chain {
        Chain { rate, eq: Equalizer::new(rate), spatial: None, head: None, fade: None, level: 1.0, faded: false }
    }

    /// The settings, where the sound goes and the volume, now.
    pub fn configure(&mut self, fx: &Fx, headphones: bool, volume: u32) {
        self.eq.set(fx.eq.gains(fx.bands, volume));
        let spatial = fx.spatial && headphones;
        if spatial && self.spatial.is_none() {
            self.spatial = Some(Spatial::new(self.rate));
            self.head = Head::listen();
        } else if !spatial && self.spatial.is_some() {
            self.spatial = None;
            self.head = None;
        }
    }

    /// The device opened again at another rate: built again for it.
    pub fn set_rate(&mut self, rate: u32) {
        if rate == self.rate {
            return;
        }
        self.rate = rate;
        let gains = self.eq.gains();
        self.eq = Equalizer::new(rate);
        self.eq.set(gains);
        if self.spatial.is_some() {
            self.spatial = Some(Spatial::new(rate));
        }
        if let Some(f) = &self.fade {
            self.fade = Some(Fade::new(f.level(), self.fade_target(), 0.0, rate));
        }
    }

    fn fade_target(&self) -> f32 {
        self.level
    }

    pub fn fade(&mut self, seconds: f32, to: f32) {
        let from = self.fade.as_ref().map_or(self.level, Fade::level);
        self.level = to;
        self.faded = false;
        self.fade = Some(Fade::new(from, to, seconds, self.rate));
    }

    /// A fade has just reached its level (said once).
    pub fn take_faded(&mut self) -> bool {
        if self.fade.as_ref().is_some_and(Fade::finished) && !self.faded {
            self.faded = true;
            // Back where it was: nothing to do any more.
            if (self.level - 1.0).abs() < 1e-4 {
                self.fade = None;
            }
            return true;
        }
        false
    }

    pub fn busy(&self) -> bool {
        !self.eq.is_flat() || self.spatial.is_some() || self.fade.is_some()
    }

    pub fn spatial(&self) -> bool {
        self.spatial.is_some()
    }

    pub fn eq(&self) -> bool {
        !self.eq.is_flat()
    }

    /// The AirPods are sending the head's pose.
    pub fn tracking(&self) -> bool {
        self.head.as_ref().is_some_and(|h| h.tracking)
    }

    pub fn process(&mut self, frames: &mut [f32]) {
        self.eq.process(frames);
        if let Some(sp) = self.spatial.as_mut() {
            if let Some(h) = self.head.as_mut() {
                if let Some((yaw, pitch)) = h.poll() {
                    sp.set_pose(yaw, pitch);
                }
            }
            sp.process(frames);
        }
        if let Some(f) = self.fade.as_mut() {
            f.process(frames);
        }
    }
}

/// The socket the shell sends the head's pose to (audiofx::head).
struct Head {
    #[cfg(target_os = "linux")]
    sock: std::os::unix::net::UnixDatagram,
    path: std::path::PathBuf,
    recenter: Recenter,
    last: Option<Instant>,
    tracking: bool,
}

impl Head {
    #[cfg(target_os = "linux")]
    fn listen() -> Option<Head> {
        let path = crate::route::run_dir().join("head.sock");
        std::fs::remove_file(&path).ok();
        let sock = std::os::unix::net::UnixDatagram::bind(&path).ok()?;
        sock.set_nonblocking(true).ok()?;
        // The shell runs as another user on a computer's build; on the
        // handheld both are root.
        crate::cleanup::remove_at_exit(&path);
        Some(Head { sock, path, recenter: Recenter::new(RECENTER_SECONDS), last: None, tracking: false })
    }

    #[cfg(not(target_os = "linux"))]
    fn listen() -> Option<Head> {
        None
    }

    /// The newest pose, relative to the stage; straight ahead again once
    /// they stop coming.
    fn poll(&mut self) -> Option<(f32, f32)> {
        #[allow(unused_mut)]
        let mut newest = None;
        #[cfg(target_os = "linux")]
        {
            let mut buf = [0u8; 16];
            while let Ok(n) = self.sock.recv(&mut buf) {
                if let Some(p) = audiofx::head::decode(&buf[..n]) {
                    newest = Some(p);
                }
            }
        }
        let now = Instant::now();
        match newest {
            Some(raw) => {
                let dt = self.last.map_or(0.0, |t| now.duration_since(t).as_secs_f32());
                self.last = Some(now);
                self.tracking = true;
                let rel = self.recenter.update(raw, dt);
                Some((rel.yaw, rel.pitch))
            }
            None if self.tracking && self.last.is_some_and(|t| now.duration_since(t).as_secs_f32() > POSES_STOPPED) => {
                self.tracking = false;
                self.recenter.reset();
                Some((0.0, 0.0))
            }
            None => None,
        }
    }
}

impl Drop for Head {
    fn drop(&mut self) {
        std::fs::remove_file(&self.path).ok();
    }
}
