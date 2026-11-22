//! Playing: the song (and the one after it, given on standard input) from
//! the decoder to the device.
//!
//! **Untouched when nothing is on.** With spatial sound, the equalizer and
//! loudness matching off (or at 0 dB), at a rate the device was opened
//! at, the decoder's samples are written as they are: to USB headphones'
//! own `hw` device at the song's rate and depth, that is bit for bit.
//! Otherwise each song is brought to the device's rate and its loudness
//! (per song), the two mixed through a crossfade, and the equalizer,
//! spatial sound and a fade run on the mix (chain.rs).
//!
//! **Gapless.** The next song is opened a few seconds before this one
//! ends, and its first frames follow this one's last in the same buffer
//! (both trimmed of the encoder's delay and padding, song.rs). At another
//! rate on a device opened at the song's own, the device is opened again
//! for it: the only gap, and a short one. **Crossfade**, if Settings ›
//! Sound has one and the songs are of different albums: the two overlap
//! at equal power for that long; `track` is said half way through.
//!
//! What it says, besides the lines main.rs lists: `track PATH` as the next
//! song takes over (`ready` and `at` follow for it), and `faded` when a
//! fade asked for gets there. /run/catalyst-music/stream says what plays
//! and how, for Settings › Sound (`rate=44100`, `bits=16`, `out_rate=`,
//! `out_bits=`, `device=hw|default`, `output=usb|bluetooth|speaker|
//! headphones`, `bitperfect=yes|no`, `processing=spatial,eq,…|none`,
//! `tracking=yes|no`).

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Instant, SystemTime};

use audiofx::loudness::{Cache, Matcher};
use audiofx::settings::Fx;

use crate::alsa::{self, Out};
use crate::chain::Chain;
use crate::control::{self, Command, Next};
use crate::resample::Resampler;
use crate::route::{self, Kind, Route};
use crate::song::Song;
use crate::{say, Fail};

/// How often the position is said while playing, seconds.
const SAY_EVERY: f64 = 5.0;
/// The next song is opened this long before this one ends (plus the crossfade).
const OPEN_AHEAD: f64 = 4.0;
/// Frames made at a time on the processed path.
const CHUNK: usize = 1024;
const FULL: f32 = 2_147_483_648.0;

pub struct PlayArgs {
    pub start: f64,
    pub size: Option<u64>,
    pub device: Option<String>,
    pub raw: bool,
    pub file: PathBuf,
    pub album: String,
    pub levels: Option<PathBuf>,
}

fn settings_path() -> PathBuf {
    PathBuf::from(std::env::var("CATALYST_AUDIO_FX").unwrap_or_else(|_| audiofx::settings::PATH.into()))
}

fn state_dir() -> PathBuf {
    PathBuf::from(std::env::var("CATALYST_AUDIO_STATE").unwrap_or_else(|_| "/data/catalyst".into()))
}

fn stream_file() -> PathBuf {
    PathBuf::from(std::env::var("CATALYST_MUSIC_RUN").unwrap_or_else(|_| "/run/catalyst-music".into())).join("stream")
}

fn loudness_cache() -> Cache {
    Cache::at(std::env::var("CATALYST_LOUDNESS").unwrap_or_else(|_| Cache::PATH.into()))
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// One song on its way to the device: at its rate, then at the device's.
struct Stream {
    song: Song,
    resampler: Option<Resampler>,
    matcher: Matcher,
    album: String,
    raw: Vec<i32>,
    /// Made, at the device's rate, not yet played.
    cooked: Vec<f32>,
    scratch: Vec<f32>,
    /// Frames played, at the device's rate.
    played: u64,
    out_rate: u32,
    done: bool,
    /// Its loudness is being measured (for the cache, at its end).
    measuring: bool,
    /// The resampler's last frames pushed out at the end; not when the
    /// next song takes the resampler on, mid-stride.
    flush_at_end: bool,
}

impl Stream {
    fn open(path: &Path, size: Option<u64>, start: f64, album: String, out_rate: u32, fx: &Fx) -> Result<Stream, Fail> {
        let song = Song::open(path, size, start)?;
        let rate = song.rate;
        let key = Cache::key(path);
        let (matcher, measuring) = match (fx.loudness, song.level.or_else(|| loudness_cache().get(&key))) {
            (false, _) => (Matcher::off(rate), false),
            (true, Some(level)) => (Matcher::known(level, rate), false),
            // Measured only from the start: a seek in hears part of it.
            (true, None) if start <= 0.0 => (Matcher::measuring(rate), true),
            (true, None) => (Matcher::off(rate), false),
        };
        Ok(Stream { resampler: (rate != out_rate).then(|| Resampler::new(rate, out_rate)), song, matcher, album, raw: Vec::with_capacity(8192), cooked: Vec::with_capacity(8192), scratch: Vec::with_capacity(8192), played: 0, out_rate, done: false, measuring, flush_at_end: true })
    }

    fn set_out_rate(&mut self, rate: u32) {
        if rate != self.out_rate {
            self.out_rate = rate;
            self.resampler = (self.song.rate != rate).then(|| Resampler::new(self.song.rate, rate));
            self.cooked.clear();
        }
    }

    /// Its samples can go to the device as they are.
    fn untouched(&self) -> bool {
        self.resampler.is_none() && self.matcher.is_unity() && self.song.output_gain_db.abs() < 0.01 && self.cooked.is_empty()
    }

    fn position(&self) -> f64 {
        self.song.start + self.played as f64 / self.out_rate as f64
    }

    /// Seconds left, if known.
    fn left(&self) -> Option<f64> {
        self.song.left().map(|f| f as f64 / self.song.rate as f64 + self.cooked.len() as f64 / 2.0 / self.out_rate as f64)
    }

    /// The raw frames of the next packet; false at the song's end.
    fn pull(&mut self) -> Result<bool, Fail> {
        if self.done {
            return Ok(false);
        }
        if !self.song.next(&mut self.raw)? {
            self.done = true;
            return Ok(false);
        }
        Ok(true)
    }

    fn cook(&mut self) {
        let gain = audiofx::db_to_gain(self.song.output_gain_db) / FULL;
        self.scratch.clear();
        self.scratch.extend(self.raw.iter().map(|&s| s as f32 * gain));
        self.matcher.process(&mut self.scratch);
        match self.resampler.as_mut() {
            Some(r) => r.process(&self.scratch, &mut self.cooked),
            None => self.cooked.extend_from_slice(&self.scratch),
        }
        self.raw.clear();
    }

    /// Up to `frames` frames at the device's rate into `out`; fewer only at
    /// the song's end.
    fn take(&mut self, frames: usize, out: &mut Vec<f32>) -> Result<usize, Fail> {
        while self.cooked.len() < frames * 2 && !self.done {
            if self.pull()? {
                self.cook();
            } else if let Some(r) = self.resampler.as_mut().filter(|_| self.flush_at_end) {
                r.finish(&mut self.cooked);
            }
        }
        let n = (self.cooked.len() / 2).min(frames);
        out.clear();
        out.extend(self.cooked.drain(..n * 2));
        Ok(n)
    }

    /// Its loudness, measured, kept for next time.
    fn remember(&self) {
        if !self.measuring || self.song.start > 0.0 {
            return;
        }
        if let Some(m) = self.matcher.meter() {
            if let Some(lufs) = m.lufs().filter(|_| m.seconds() >= 30.0) {
                loudness_cache().put(&Cache::key(&self.song.path), lufs, m.peak()).ok();
            }
        }
    }
}

/// The device, opened as the route says for a song.
struct Device {
    out: Box<dyn Out>,
    exact: bool,
    bit_perfect: bool,
    name: String,
}

fn open_device(a: &PlayArgs, route: &Route, song: &Song) -> Result<Device, Fail> {
    if a.raw {
        return Ok(Device { out: Box::new(alsa::Raw::new(crate::RATE)), exact: false, bit_perfect: false, name: "raw".into() });
    }
    let mut last = None;
    for way in route::plan(route, song.rate, song.bits, a.device.as_deref()) {
        match alsa::Alsa::open(&way.device, way.rate, way.format, way.exact) {
            Ok(out) => {
                let bit_perfect = way.bit_perfect(song.bits) && way.rate == song.rate;
                let name = if way.exact { "hw" } else { "default" };
                return Ok(Device { out: Box::new(out), exact: way.exact, bit_perfect, name: name.into() });
            }
            Err(e) => last = Some(e),
        }
    }
    let e = last.unwrap_or_else(|| "no sound device".into());
    Err(if e.contains("busy") { Fail::Busy(e) } else { Fail::Other(e) })
}

/// What changes while it plays: the settings, the volume (for the
/// loudness preset), and whether headphones are what it plays through.
struct Watch {
    fx_path: PathBuf,
    fx_time: Option<SystemTime>,
    fx: Fx,
    volume_path: PathBuf,
    volume_time: Option<SystemTime>,
    volume: u32,
    route: Route,
    jack: bool,
    looked: Instant,
    jack_looked: Instant,
}

impl Watch {
    fn new(route: Route) -> Watch {
        let fx_path = settings_path();
        let volume_path = state_dir().join(if route.kind == Kind::Usb { "audio-volume-usb" } else { "audio-volume" });
        let mut w = Watch { fx: audiofx::settings::load(&fx_path), fx_time: modified(&fx_path), fx_path, volume_time: None, volume_path, volume: 65, route, jack: false, looked: Instant::now(), jack_looked: Instant::now() };
        w.read_volume();
        w.read_jack();
        w
    }

    fn read_volume(&mut self) {
        self.volume_time = modified(&self.volume_path);
        self.volume = std::fs::read_to_string(&self.volume_path).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(if self.route.kind == Kind::Usb { 40 } else { 65 });
    }

    fn read_jack(&mut self) {
        self.jack_looked = Instant::now();
        // Only asked while it matters (spatial sound on, the codec playing).
        self.jack = self.route.kind == Kind::Local && self.fx.spatial && alsa::jack_in("ES8389Audio").unwrap_or(false);
    }

    /// Headphones, not a speaker: USB, Bluetooth headphones, the jack.
    fn headphones(&self) -> bool {
        if std::env::var_os("CATALYST_PLAY_HEADPHONES").is_some() {
            return true;
        }
        match self.route.kind {
            Kind::Usb => true,
            Kind::Bluetooth => !self.route.bluetooth_speaker,
            Kind::Local => self.jack,
        }
    }

    /// Looked at twice a second: true if anything changed.
    fn changed(&mut self) -> bool {
        if self.looked.elapsed().as_millis() < 500 {
            return false;
        }
        self.looked = Instant::now();
        let mut changed = false;
        let t = modified(&self.fx_path);
        if t != self.fx_time {
            self.fx_time = t;
            self.fx = audiofx::settings::load(&self.fx_path);
            changed = true;
        }
        if modified(&self.volume_path) != self.volume_time {
            self.read_volume();
            changed = true;
        }
        if self.jack_looked.elapsed().as_secs() >= 2 {
            let was = self.jack;
            self.read_jack();
            changed |= was != self.jack;
        }
        changed
    }
}

/// The level file the island reads (audiofx::levels).
struct Levels {
    file: std::fs::File,
    seq: u16,
}

impl Levels {
    fn open(path: &Path) -> Option<Levels> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let file = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(path).ok()?;
        crate::cleanup::remove_at_exit(path);
        Some(Levels { file, seq: 0 })
    }

    fn put(&mut self, left: f32, right: f32) {
        self.seq = self.seq.wrapping_add(1);
        let b = audiofx::levels::encode(self.seq, left, right);
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::FileExt;
            self.file.write_at(&b, 0).ok();
        }
        #[cfg(not(target_os = "linux"))]
        {
            use std::io::{Seek, Write};
            self.file.seek(std::io::SeekFrom::Start(0)).ok();
            self.file.write_all(&b).ok();
        }
    }
}

fn rms_i32(frames: &[i32]) -> (f32, f32) {
    let n = (frames.len() / 2).max(1) as f32;
    let (mut l, mut r) = (0.0f32, 0.0f32);
    for f in frames.chunks_exact(2) {
        let (a, b) = (f[0] as f32 / FULL, f[1] as f32 / FULL);
        l += a * a;
        r += b * b;
    }
    ((l / n).sqrt(), (r / n).sqrt())
}

fn output_word(route: &Route, headphones: bool) -> &'static str {
    match route.kind {
        Kind::Usb => "usb",
        Kind::Bluetooth => "bluetooth",
        Kind::Local if headphones => "headphones",
        Kind::Local => "speaker",
    }
}

/// /run/catalyst-music/stream, when it says something new.
struct Status {
    path: PathBuf,
    said: String,
}

impl Status {
    fn put(&mut self, text: String) {
        if text == self.said {
            return;
        }
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let tmp = self.path.with_extension("new");
        if std::fs::write(&tmp, &text).is_ok() && std::fs::rename(&tmp, &self.path).is_ok() {
            crate::cleanup::remove_at_exit(&self.path);
        }
        self.said = text;
    }
}

fn say_ready(s: &Stream) {
    let song = &s.song;
    say(&format!("ready codec={} rate={} channels={} seconds={:.1}", song.codec, song.rate, song.channels, song.seconds));
    say(&format!("at {:.1}", song.start));
}

pub fn run(a: &PlayArgs) -> Result<(), Fail> {
    let commands: Receiver<Command> = control::listen();
    let route = if a.raw { route::parse("local", "", "") } else { route::read(&route::run_dir()) };
    let mut watch = Watch::new(route.clone());
    let mut cur = Stream::open(&a.file, a.size, a.start, a.album.clone(), crate::RATE, &watch.fx)?;
    let mut dev = open_device(a, &route, &cur.song)?;
    cur.set_out_rate(dev.out.rate());
    let mut chain = Chain::new(dev.out.rate());
    chain.configure(&watch.fx, watch.headphones(), watch.volume);
    let mut levels = a.levels.as_deref().and_then(Levels::open);
    let mut status = Status { path: stream_file(), said: String::new() };

    let mut next_spec: Option<Next> = None;
    let mut next: Option<Stream> = None;
    // A crossfade under way: its length and how far it is (device frames),
    // and whether the next song has been said to have taken over.
    let mut xfade: Option<(u64, u64, bool)> = None;
    let mut buf: Vec<f32> = Vec::with_capacity(CHUNK * 2);
    let mut buf2: Vec<f32> = Vec::with_capacity(CHUNK * 2);
    let mut ints: Vec<i32> = Vec::with_capacity(CHUNK * 2);
    let mut next_say = cur.song.start + SAY_EVERY;
    let mut xruns_said = 0;
    say_ready(&cur);

    loop {
        // What the shell said.
        while let Ok(c) = commands.try_recv() {
            match c {
                Command::Next(n) => {
                    // A crossfade begun is past changing.
                    if xfade.is_none() {
                        if next.as_ref().map(|s| &s.song.path) != n.as_ref().map(|n| &n.path) {
                            next = None;
                        }
                        next_spec = n;
                    }
                }
                Command::Album(k) => cur.album = k,
                Command::Fade { seconds, to } => chain.fade(seconds, to),
            }
        }
        if watch.changed() {
            chain.configure(&watch.fx, watch.headphones(), watch.volume);
        }
        if chain.take_faded() {
            say("faded");
        }

        // The next song opened ahead; a crossfade begun when it is time.
        let overlap = match (&next_spec, &next) {
            (_, Some(n)) => audiofx::crossfade::overlap(watch.fx.crossfade, audiofx::crossfade::same_album(&cur.album, &n.album), Some(cur.song.seconds as f32), Some(n.song.seconds as f32)) as f64,
            _ => watch.fx.crossfade as f64,
        };
        if next.is_none() && xfade.is_none() && next_spec.is_some() && (cur.left().is_some_and(|l| l <= overlap + OPEN_AHEAD) || cur.done) {
            next = open_next(&mut next_spec, dev.out.rate(), &watch.fx);
        }
        if xfade.is_none() && overlap > 0.0 && next.is_some() && cur.left().is_some_and(|l| l <= overlap) {
            xfade = Some(((overlap * dev.out.rate() as f64) as u64, 0, false));
        }
        // Gapless at one rate: the resampler is handed on, not emptied.
        cur.flush_at_end = !(xfade.is_none() && !dev.exact && next.as_ref().is_some_and(|n| n.song.rate == cur.song.rate));

        // A crossfade to come is begun on its frame: made, not copied.
        let fade_due = xfade.is_none() && overlap > 0.0 && next.is_some();
        let direct = !fade_due && xfade.is_none() && !chain.busy() && cur.untouched() && cur.song.rate == dev.out.rate();
        if direct {
            if !cur.pull()? {
                if next.is_none() {
                    next = open_next(&mut next_spec, dev.out.rate(), &watch.fx);
                }
                if !end_of_song(a, &route, &mut cur, &mut next, &mut xfade, &mut dev, &mut chain, &watch)? {
                    break;
                }
                next_say = cur.position() + SAY_EVERY;
                continue;
            }
            dev.out.write(&cur.raw).map_err(Fail::Other)?;
            if let Some(l) = levels.as_mut() {
                let (left, right) = rms_i32(&cur.raw);
                l.put(left, right);
            }
            cur.played += cur.raw.len() as u64 / 2;
            cur.raw.clear();
        } else {
            let until = cur.left().filter(|_| fade_due).map(|l| ((l - overlap) * dev.out.rate() as f64).round().max(0.0) as usize);
            let n = cur.take(until.filter(|&u| u > 0).map_or(CHUNK, |u| u.min(CHUNK)), &mut buf)?;
            if n == 0 {
                if next.is_none() {
                    next = open_next(&mut next_spec, dev.out.rate(), &watch.fx);
                }
                if !end_of_song(a, &route, &mut cur, &mut next, &mut xfade, &mut dev, &mut chain, &watch)? {
                    break;
                }
                next_say = cur.position() + SAY_EVERY;
                continue;
            }
            cur.played += n as u64;
            if let Some((total, done, said)) = xfade.as_mut() {
                // The next song in under this one's end, at equal power.
                if let Some(nx) = next.as_mut() {
                    let m = nx.take(n, &mut buf2)?;
                    nx.played += m as u64;
                    buf2.resize(n * 2, 0.0);
                    if m < n {
                        buf2[m * 2..].fill(0.0);
                    }
                    for i in 0..n {
                        let t = (*done + i as u64) as f32 / (*total).max(1) as f32;
                        let (g_out, g_in) = audiofx::crossfade::gains(t);
                        buf[i * 2] = buf[i * 2] * g_out + buf2[i * 2] * g_in;
                        buf[i * 2 + 1] = buf[i * 2 + 1] * g_out + buf2[i * 2 + 1] * g_in;
                    }
                    *done += n as u64;
                    if !*said && *done * 2 >= *total {
                        *said = true;
                        say(&format!("track {}", nx.song.path.display()));
                        say_ready(nx);
                        next_say = nx.position() + SAY_EVERY;
                    }
                }
                // Through: the old song let go, the new one plays on alone.
                if *done >= *total {
                    let said = *said;
                    if let Some(nx) = next.take() {
                        cur.remember();
                        cur = nx;
                        if !said {
                            say(&format!("track {}", cur.song.path.display()));
                            say_ready(&cur);
                            next_say = cur.position() + SAY_EVERY;
                        }
                    }
                    xfade = None;
                }
            }
            // The soft clip only where something can lift the sound; else
            // the samples go back exactly as they came (a gain under 0 dB
            // never needs it).
            let lifts = chain.busy() || cur.matcher.gain_db() > 0.0 || cur.song.output_gain_db > 0.0 || xfade.is_some();
            chain.process(&mut buf[..n * 2]);
            to_ints(&buf[..n * 2], lifts, &mut ints);
            dev.out.write(&ints).map_err(Fail::Other)?;
            if let Some(l) = levels.as_mut() {
                let (left, right) = audiofx::levels::rms(&buf[..n * 2]);
                l.put(left, right);
            }
        }

        // Said: where the song is, the device running dry, how it plays.
        let reported = match (&xfade, &next) {
            (Some((_, _, true)), Some(nx)) => nx,
            _ => &cur,
        };
        let at = reported.position();
        if at >= next_say {
            next_say = at + SAY_EVERY;
            say(&format!("at {at:.1}"));
        }
        if dev.out.xruns() != xruns_said {
            xruns_said = dev.out.xruns();
            say(&format!("xrun {xruns_said}"));
        }
        if !a.raw {
            let processing: Vec<&str> = [(chain.spatial(), "spatial"), (chain.eq(), "eq"), (!reported.matcher.is_unity(), "loudness"), (xfade.is_some(), "crossfade"), (reported.resampler.is_some(), "resampled")].iter().filter(|p| p.0).map(|p| p.1).collect();
            let bit_perfect = dev.bit_perfect && direct;
            status.put(format!(
                "codec={}\nrate={}\nbits={}\nout_rate={}\nout_bits={}\ndevice={}\noutput={}\nbitperfect={}\nprocessing={}\ntracking={}\ngain={:.1}\n",
                reported.song.codec,
                reported.song.rate,
                reported.song.bits,
                dev.out.rate(),
                dev.out.format().bits(),
                dev.name,
                output_word(&watch.route, watch.headphones()),
                if bit_perfect { "yes" } else { "no" },
                if processing.is_empty() { "none".to_owned() } else { processing.join(",") },
                if chain.tracking() { "yes" } else { "no" },
                reported.matcher.gain_db(),
            ));
        }
    }
    cur.remember();
    dev.out.drain();
    crate::cleanup::now();
    say("end");
    Ok(())
}

/// Float frames back to the device's integers: through the soft clip if
/// they may have been lifted, else only held in bounds.
fn to_ints(frames: &[f32], soft: bool, out: &mut Vec<i32>) {
    out.clear();
    if soft {
        out.extend(frames.iter().map(|&x| (audiofx::soft_clip(x) * FULL) as i32));
    } else {
        // `as` saturates: 1.0 comes back as i32::MAX.
        out.extend(frames.iter().map(|&x| (x * FULL) as i32));
    }
}

/// The song given as next, opened (or said to be unreadable).
fn open_next(spec: &mut Option<Next>, out_rate: u32, fx: &Fx) -> Option<Stream> {
    let spec = spec.take()?;
    match Stream::open(&spec.path, spec.size, 0.0, spec.album, out_rate, fx) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("catalyst-play: the next song: {}", e.words());
            say("next unreadable");
            None
        }
    }
}

/// This song has ended: on into the next (the device opened again for it
/// if it must be), or the end. False: nothing more to play.
#[allow(clippy::too_many_arguments)]
fn end_of_song(a: &PlayArgs, route: &Route, cur: &mut Stream, next: &mut Option<Stream>, xfade: &mut Option<(u64, u64, bool)>, dev: &mut Device, chain: &mut Chain, watch: &Watch) -> Result<bool, Fail> {
    let Some(mut nx) = next.take() else { return Ok(false) };
    cur.remember();
    let said = xfade.as_ref().is_some_and(|x| x.2);
    // Mid-crossfade, the old song came to its end first: the new one plays
    // on alone, its fade-in no longer needed.
    *xfade = None;
    if dev.exact && (nx.song.rate != dev.out.rate() || nx.song.bits > dev.out.format().bits()) {
        // A device opened at the song's own rate: opened again at the next one's.
        dev.out.drain();
        let fresh = open_device(a, route, &nx.song)?;
        *dev = fresh;
        nx.set_out_rate(dev.out.rate());
        chain.set_rate(dev.out.rate());
        chain.configure(&watch.fx, watch.headphones(), watch.volume);
    } else if !cur.flush_at_end && nx.song.rate == cur.song.rate && nx.played == 0 && nx.cooked.is_empty() {
        // The same rate: the resampler runs on through the join, so there
        // is not even a filter's length of silence between them.
        nx.resampler = cur.resampler.take();
    }
    *cur = nx;
    if !said {
        say(&format!("track {}", cur.song.path.display()));
        say_ready(cur);
    }
    Ok(true)
}
