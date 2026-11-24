//! The time-lapse (1.5.1): a frame every few seconds for as long as it
//! runs, kept as a video that plays round and round in Photos and goes
//! into the build log as photos do: a robot's build session in a minute.
//!
//! - **Every** (`s`, or its chip: 2, 5, 10, 30 s or a minute; 5 s to begin
//!   with, kept). Its chip says what an hour comes to ("1 h plays in 48 s":
//!   lapse.rs's playback_fps).
//! - **Enter starts it**: a frame at once, then one on each beat (a beat
//!   the camera is still busy for is let go, not made up), each the
//!   camera's own still at 960 × 720 (catalyst-camera's `size=`, quality
//!   85), its exposure the camera's as it is then, so a room's lights going
//!   on or off are followed. The screen dims after a few seconds and the
//!   handheld stays awake, as the intervalometer's run does.
//! - **Enter or Esc stops it**, keeping every frame taken: the one being
//!   taken is waited for, then the frames are made into the AVI on a
//!   thread (lapse.rs: one frame read at a time). One frame alone is kept
//!   as a photo. It stops by itself at 1800 frames (lapse.rs's MOST_FRAMES).
//!   The Camera closed while one runs makes it all the same, and frames left
//!   by anything else (the battery) are made into it the next time the
//!   Camera opens.
//! - **The build log**: its moment is noted as it starts (the Wi-Fi's place,
//!   the event, photolog/), and the album lists it with the photos.
//! - **Memory**: catalyst-camera holds one 960 × 720 frame (1 MB) while it
//!   takes it; the shell holds nothing but its count.

use super::*;
use crate::lapse::{self, Made};

/// A time-lapse being taken.
pub(super) struct Taking {
    /// The video it becomes, and where its frames wait meanwhile.
    pub path: PathBuf,
    pub work: PathBuf,
    pub every: Duration,
    pub began: Instant,
    /// When the next frame is due.
    pub next: Instant,
    /// Frames asked of the camera, and those it has kept.
    pub asked: u32,
    pub saved: u32,
    /// The frame being taken.
    pub pending: Option<PathBuf>,
    /// Stopped, and since when (the frame being taken is waited for).
    pub stopping: Option<Instant>,
}

/// How long a frame being taken is waited for once stopped, before the
/// video is made without it.
const WAIT_LAST: Duration = Duration::from_secs(12);

impl Taking {
    /// Started now: the first frame at once.
    pub fn start(path: PathBuf, every_s: u32, now: Instant) -> Taking {
        let work = lapse::work_of(&path);
        Taking { path, work, every: Duration::from_secs(u64::from(every_s.max(1))), began: now, next: now, asked: 0, saved: 0, pending: None, stopping: None }
    }

    /// A frame is due (and none is being taken, and it is not stopped or full).
    pub fn due(&self, now: Instant) -> bool {
        self.stopping.is_none() && self.pending.is_none() && now >= self.next && !self.full()
    }

    pub fn full(&self) -> bool {
        self.asked >= lapse::MOST_FRAMES
    }

    /// A frame asked for at `now`: its file; the next on the beat (beats
    /// missed while the camera was busy let go, not made up).
    pub fn ask(&mut self, now: Instant) -> PathBuf {
        self.asked += 1;
        let path = lapse::frame_path(&self.work, self.asked);
        self.pending = Some(path.clone());
        self.next += self.every;
        while self.next <= now {
            self.next += self.every;
        }
        path
    }

    /// The frame being taken is in (or never will be).
    pub fn landed(&mut self, kept: bool) {
        if kept {
            self.saved += 1;
        }
        self.pending = None;
    }

    /// Stopped: ready to be made once the frame being taken is in (or has
    /// been waited for long enough).
    pub fn ready(&self, now: Instant) -> bool {
        self.stopping.is_some_and(|at| self.pending.is_none() || now.saturating_duration_since(at) > WAIT_LAST)
    }

    /// "24 frames · 2 min" (and how soon the next).
    pub fn words(&self, now: Instant) -> (String, String) {
        let frames = if self.saved == 1 { "1 frame".to_owned() } else { format!("{} frames", self.saved) };
        let run = camkit::duration_words(now.saturating_duration_since(self.began).as_secs() as u32);
        let next = self.next.saturating_duration_since(now).as_secs_f32().ceil() as u32;
        (format!("{frames} · {run}"), format!("next in {}", camkit::duration_words(next)))
    }

    /// How far to the next frame (0 to 1).
    pub fn toward_next(&self, now: Instant) -> f32 {
        1.0 - self.next.saturating_duration_since(now).as_secs_f32() / self.every.as_secs_f32().max(0.001)
    }
}

/// What a time-lapse made says.
pub fn made_words(made: &Made) -> String {
    match made {
        Made::Lapse { frames, .. } => format!("time-lapse kept · {frames} frames, plays {}", lapse::seconds_words(lapse::plays_for(*frames))),
        Made::Photo(_) => "one frame taken: kept as a photo".to_owned(),
        Made::Nothing => "time-lapse: no frames were taken".to_owned(),
    }
}

impl Camera {
    /// Enter in lapse mode: a time-lapse started, or stopped (kept).
    pub(super) fn toggle_lapse(&mut self, calm: bool) {
        if self.lapse.is_some() {
            return self.stop_lapse();
        }
        if self.preview.is_none() || self.metered.is_none() {
            self.say("the camera is not ready yet");
            return;
        }
        let path = lapse::new_path(chrono::Local::now().naive_local());
        let taking = Taking::start(path.clone(), self.lapse_every, Instant::now());
        if std::fs::create_dir_all(&taking.work).is_err() {
            self.say("the card would not take a time-lapse");
            return;
        }
        // Its first moment, as the thumbnail, at once.
        self.instant_shot(&path, true);
        self.lapse = Some(taking);
        self.bar.focus = None;
        self.bar.hidden = true;
        self.press.jump(1.0);
        self.press.set(0.0);
        if !calm {
            self.flash.jump(0.6);
            self.flash.set(0.0);
        }
        self.say(format!("time-lapse · a frame every {} · enter: stop", camkit::duration_words(self.lapse_every)));
    }

    /// Stopped: the frame being taken waited for, then made.
    pub(super) fn stop_lapse(&mut self) {
        if let Some(t) = &mut self.lapse {
            if t.stopping.is_none() {
                t.stopping = Some(Instant::now());
                self.say("finishing the time-lapse…");
            }
        }
    }

    /// What the time-lapse needs now (from `update`): the next frame when it
    /// is due and the camera has the last photo's picture; made once stopped
    /// (or full). True if anything changed.
    pub(super) fn tick_lapse(&mut self, now: Instant) -> bool {
        let busy = self.saving.is_some() && !self.captured;
        let Some(t) = &mut self.lapse else { return false };
        if t.full() && t.pending.is_none() && t.stopping.is_none() {
            t.stopping = Some(now);
        }
        if t.ready(now) {
            self.make_lapse();
            return true;
        }
        if t.due(now) && !busy && self.preview.is_some() {
            self.lapse_shot(now);
            return true;
        }
        false
    }

    /// A frame: the camera's own still at the time-lapse's size, into its
    /// frames' folder, timed as photos are (so one that never comes is given
    /// up on).
    fn lapse_shot(&mut self, now: Instant) {
        let Some(m) = self.metered else { return };
        let Some(t) = &mut self.lapse else { return };
        let path = t.ask(now);
        let mut words = pro::still(None, &m, false).words();
        words.push(format!("size={}x{}", lapse::FRAME.0, lapse::FRAME.1));
        words.push(format!("quality={}", lapse::QUALITY));
        if !self.control.send(Order::Still(path.clone(), words)) {
            t.landed(false);
            return;
        }
        let mut timeline = Timeline::new(path.clone(), now);
        timeline.mark(Mark::Request, Instant::now());
        self.kit.shot = Some(timeline);
        self.saving = Some(now);
        self.captured = false;
        self.pending += 1;
        self.pending_paths.push(path);
    }

    /// A frame saved (or failed): the time-lapse's, if it is one of its
    /// frames (true: nothing more of it for the album).
    pub(super) fn lapse_frame_saved(&mut self, path: &std::path::Path) -> bool {
        if !path.starts_with(lapse::work_root()) && !self.lapse.as_ref().is_some_and(|t| path.starts_with(&t.work)) {
            return false;
        }
        self.pending = self.pending.saturating_sub(1);
        self.pending_paths.retain(|p| p != path);
        if self.pending == 0 {
            self.saving = None;
        }
        if let Some(at) = self.kit.writing.iter().position(|t| t.path == path) {
            let mut t = self.kit.writing.remove(at);
            t.mark(Mark::Written, Instant::now());
            eprintln!("{}", t.log_line());
        }
        match &mut self.lapse {
            Some(t) if t.pending.as_deref() == Some(path) => t.landed(true),
            // A frame of one already made (it came after): not kept twice.
            _ => {
                std::fs::remove_file(path).ok();
            }
        }
        true
    }

    /// A photo failed while a time-lapse's frame was being taken: that
    /// frame let go, the time-lapse going on.
    pub(super) fn lapse_frame_failed(&mut self) {
        if let Some(t) = &mut self.lapse {
            if t.pending.is_some() {
                t.landed(false);
            }
        }
    }

    /// Its frames made into the video (or a photo) on a thread; the Camera
    /// told when it is done.
    pub(super) fn make_lapse(&mut self) {
        let Some(t) = self.lapse.take() else { return };
        self.pending_paths.retain(|p| !p.starts_with(&t.work));
        self.bar.hidden = false;
        let live = self.live.clone();
        let path = t.path;
        std::thread::Builder::new()
            .name("lapse-make".into())
            .spawn(move || {
                crate::sched::enter(crate::sched::Class::Background);
                let made = lapse::assemble(&path);
                live.lock().map(|mut l| l.events.push(Event::LapseMade(made))).ok();
                crate::platform::wake();
            })
            .ok();
    }

    /// The time-lapse made: in the album, its thumbnail in the corner, said.
    pub(super) fn lapse_made(&mut self, made: Result<Made, String>) {
        match made {
            Ok(made) => {
                photos::changed();
                crate::photolog::changed();
                match &made {
                    Made::Lapse { path, .. } | Made::Photo(path) => self.load_last(Some(path.clone())),
                    Made::Nothing => {
                        self.instant = None;
                        self.load_last(None);
                    }
                }
                self.say(made_words(&made));
            }
            Err(why) => self.say(why),
        }
    }

    /// The interval a step along its choices (`s`), kept.
    pub(super) fn step_lapse_every(&mut self) {
        let at = lapse::EVERY.iter().position(|s| *s == self.lapse_every).unwrap_or(1);
        self.set_lapse_every(lapse::EVERY[(at + 1) % lapse::EVERY.len()]);
    }

    pub(super) fn set_lapse_every(&mut self, s: u32) {
        self.lapse_every = s;
        self.keep_prefs();
        self.say(format!("a frame every {} · {}", camkit::duration_words(s), lapse::hour_words(s)));
    }

    /// Its chip's words: "every 5 s · 1 h plays in 48 s".
    pub(super) fn lapse_words_now(&self) -> String {
        format!("every {} · {}", camkit::duration_words(self.lapse_every), lapse::hour_words(self.lapse_every))
    }

    /// While a time-lapse is taken: a chip at the top with its breathing
    /// dot, its frames and how long it has run; at the foot, the next
    /// frame's seconds and how to stop.
    pub(super) fn draw_lapse(&mut self, g: &mut Gfx, vf: Rect, top: f32) {
        let Some(t) = &self.lapse else { return };
        let (tokens, cap) = (g.tokens, g.ty.caption);
        let white = Color(255, 255, 255);
        let now = Instant::now();
        let m = 6.0;
        let h = cap.size + 8.0;
        let (count, next) = t.words(now);
        let stopping = t.stopping.is_some();
        let forms = if stopping { vec!["finishing the time-lapse…".to_owned(), "finishing…".to_owned()] } else { vec![format!("time-lapse · {count}"), count.clone(), format!("{} frames", t.saved)] };
        let room = vf.w - 2.0 * m - 20.0;
        let words = forms.iter().find(|f| g.measure(f, cap) <= room).unwrap_or(&forms[forms.len() - 1]).clone();
        let dot = 6.0;
        let w = (g.measure(&words, cap) + 16.0 + dot + 6.0).round();
        let y = (vf.y + m).max(top);
        g.canvas.rrect(Rect::new(vf.x + m + g.ox, y, w, h), h * 0.5, Color(0, 0, 0), 0.55);
        // The dot breathes once a second (the run is alive), still when calm.
        let secs = now.saturating_duration_since(t.began).as_secs_f32();
        let breathe = if g.calm { 1.0 } else { 0.65 + 0.35 * (secs * std::f32::consts::PI).cos().abs() };
        g.canvas.circle(vf.x + m + 8.0 + dot * 0.5 + g.ox, y + h * 0.5, dot * 0.5, tokens.signal, breathe);
        g.text_alpha(vf.x + m + 8.0 + dot + 6.0, y + h * 0.5, &words, cap, white, 1.0);
        if !stopping {
            let forms = [format!("{next} · enter or esc: stop"), format!("{next} · enter: stop"), next.clone()];
            let hint = forms.iter().find(|f| g.measure(f, cap) + 16.0 <= vf.w - 2.0 * m).unwrap_or(&forms[2]).clone();
            let hw = g.measure(&hint, cap) + 16.0;
            let hy = vf.bottom() - m - h;
            g.canvas.rrect(Rect::new(vf.cx() - hw * 0.5 + g.ox, hy, hw, h), h * 0.5, Color(0, 0, 0), 0.55);
            g.text_alpha(vf.cx() - hw * 0.5 + 8.0, hy + h * 0.5, &hint, cap, white, 1.0);
        }
    }

    /// At rest in burst, trails or lapse mode: what a press of the shutter
    /// will take, in a chip at the top of the picture (as the look's is).
    pub(super) fn draw_mode_chip(&mut self, g: &mut Gfx, vf: Rect, alpha: f32) {
        if alpha <= 0.01 || self.lapse.is_some() || self.stacking.is_some() {
            return;
        }
        let t = g.tokens;
        let (icon, words) = match self.mode {
            Mode::Burst => ("burst_mode", self.burst_words_now()),
            Mode::Trails => ("gesture", self.trails_words_now()),
            Mode::Lapse => ("timelapse", self.lapse_words_now()),
            _ => return,
        };
        let cap = g.ty.caption;
        let room = vf.w - 12.0 - 14.0 - cap.size - 2.0;
        let words = if g.measure(&words, cap) <= room { words } else { g.text.fit(words.split(" · ").next().unwrap_or(&words), &cap, room) };
        Self::chip(g, vf.x + 6.0, vf.y + 6.0, Some(icon), &words, t.signal, alpha);
    }
}

/// The simulator's smaller still (a time-lapse's frame): its scene at the
/// size asked, exposed as told, written at the quality asked.
pub(super) fn sim_small(live: &Arc<Mutex<Live>>, path: PathBuf, t: f32, words: &[String]) {
    let (w, h) = still_word(words, "size")
        .and_then(|v| v.split_once('x'))
        .and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?)))
        .map_or((960, 720), |(a, b)| (a & !1, b & !1));
    let exposure = still_word(words, "exp").and_then(|v| v.parse::<f32>().ok());
    let gain = still_word(words, "gain").and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
    let ct = still_word(words, "ct").and_then(|v| v.parse().ok()).unwrap_or(SIM_CT);
    let tell = |e: Event| {
        live.lock().map(|mut l| l.events.push(e)).ok();
        crate::platform::wake();
    };
    tell(Event::Exposing("133 1 still".into()));
    let mut pic = scene(w, h, t, false);
    sim_expose(&mut pic, exposure.map_or(1.0, |e| e * gain / sim_needs(0.0)), ct);
    let yuv = rgb_to_yuv420(&pic);
    tell(Event::Exposed);
    tell(Event::Captured("room 0 stop 0 config 0 start 0 frame 120 copy 120 captured 120".into()));
    let live = live.clone();
    std::thread::Builder::new()
        .name("photo-write".into())
        .spawn(move || {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).ok();
            }
            let event = match crate::burst::write_jpeg(&path, &yuv, w, h, None) {
                Ok(()) => Event::Saved { path, size: (w as u32, h as u32), detail: false, raw_skipped: false, stacked: None },
                Err(why) => Event::Failed(why),
            };
            live.lock().map(|mut l| l.events.push(event)).ok();
            crate::platform::wake();
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_come_on_the_beat_and_a_missed_beat_is_let_go() {
        let t0 = Instant::now();
        let mut t = Taking::start(PathBuf::from("/p/LAPSE_1.avi"), 5, t0);
        assert_eq!(t.work, PathBuf::from("/p/.lapse/LAPSE_1"));
        assert!(t.due(t0), "the first at once");
        assert_eq!(t.ask(t0), PathBuf::from("/p/.lapse/LAPSE_1/0001.jpg"));
        assert!(!t.due(t0 + Duration::from_secs(6)), "not while one is being taken");
        t.landed(true);
        assert!(!t.due(t0 + Duration::from_secs(4)) && t.due(t0 + Duration::from_secs(5)));
        // The camera busy for two beats: the next on the beat after, not three at once.
        let late = t0 + Duration::from_secs(16);
        assert_eq!(t.ask(late).file_name().unwrap(), "0002.jpg");
        assert_eq!(t.next, t0 + Duration::from_secs(20));
        t.landed(false);
        assert_eq!((t.asked, t.saved), (2, 1), "a frame that failed is not counted");
        assert!((t.toward_next(t0 + Duration::from_secs(18)) - 0.6).abs() < 0.01);
        let (count, next) = t.words(t0 + Duration::from_secs(18));
        assert_eq!((count.as_str(), next.as_str()), ("1 frame · 18 s", "next in 2 s"));
    }

    #[test]
    fn stopped_it_waits_for_the_frame_being_taken_then_is_made() {
        let t0 = Instant::now();
        let mut t = Taking::start(PathBuf::from("/p/LAPSE_1.avi"), 2, t0);
        t.ask(t0);
        t.stopping = Some(t0 + Duration::from_secs(1));
        assert!(!t.due(t0 + Duration::from_secs(3)), "no frame once stopped");
        assert!(!t.ready(t0 + Duration::from_secs(2)), "the frame being taken is waited for");
        assert!(t.ready(t0 + Duration::from_secs(1) + WAIT_LAST + Duration::from_millis(1)), "but not for ever");
        t.landed(true);
        assert!(t.ready(t0 + Duration::from_secs(2)));
        // Full: no more asked.
        let mut full = Taking::start(PathBuf::from("/p/LAPSE_2.avi"), 2, t0);
        full.asked = lapse::MOST_FRAMES;
        assert!(full.full() && !full.due(t0 + Duration::from_secs(100)));
        assert_eq!(made_words(&Made::Lapse { path: PathBuf::new(), frames: 40, size: (960, 720) }), "time-lapse kept · 40 frames, plays 8 s");
        assert_eq!(made_words(&Made::Photo(PathBuf::new())), "one frame taken: kept as a photo");
    }

    fn key(camera: &mut Camera, key: Key) {
        let (mut robot, mut settings, mut say) = (Robot::none(), crate::shell::Settings::default(), None);
        camera.key(key, &mut Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true });
    }

    /// Ticked and updated as the shell does, until `done` or `ms` pass.
    fn run_until(camera: &mut Camera, ms: u64, done: impl Fn(&Camera) -> bool) {
        let until = Instant::now() + Duration::from_millis(ms);
        let (mut robot, mut settings, mut say) = (Robot::none(), crate::shell::Settings::default(), None);
        while Instant::now() < until && !done(camera) {
            std::thread::sleep(Duration::from_millis(16));
            camera.tick(1.0 / 60.0);
            camera.ambient_tick(1.0 / 60.0, &robot);
            camera.update(&mut Cx { robot: &mut robot, settings: &mut settings, say: &mut say, at_rest: true });
        }
    }

    fn ready_in(mode: Mode) -> Camera {
        crate::shell::tests::sandbox();
        Prefs::default().save();
        let mut camera = Camera::new();
        run_until(&mut camera, 5000, |c| c.metered.is_some() && c.preview.is_some());
        camera.set_mode(mode);
        camera
    }

    #[test]
    fn a_burst_keeps_its_sharpest_frame_and_esc_ends_it_keeping_that() {
        let _port = crate::smartcam::sim::exclusive(false);
        let mut camera = ready_in(Mode::Burst);
        key(&mut camera, Key::Enter);
        assert!(camera.stacking.as_ref().is_some_and(|s| s.live && s.what == camkit::Exposing::Burst), "its ring");
        run_until(&mut camera, 20_000, |c| c.saving.is_none());
        let note = camera.note.as_ref().map(|(n, _)| n.clone()).unwrap_or_default();
        assert!(note.starts_with("burst · frame 6 of 10"), "{note}");
        // Esc part way: what it has, kept.
        key(&mut camera, Key::Enter);
        run_until(&mut camera, 3000, |c| c.stacking.as_ref().is_some_and(|s| s.taken() >= 2));
        key(&mut camera, Key::Back);
        run_until(&mut camera, 20_000, |c| c.saving.is_none());
        let note = camera.note.as_ref().map(|(n, _)| n.clone()).unwrap_or_default();
        assert!(note.starts_with("burst · frame") && !note.contains("of 10"), "kept, fewer than ten: {note}");
    }

    #[test]
    fn light_trails_build_live_and_enter_finishes_them_kept() {
        let _port = crate::smartcam::sim::exclusive(false);
        let mut camera = ready_in(Mode::Trails);
        let seen = camera.seen;
        key(&mut camera, Key::Enter);
        assert!(camera.trails_running());
        run_until(&mut camera, 3000, |c| c.stacking.as_ref().is_some_and(|s| s.taken() >= 8));
        assert!(camera.seen > seen, "the trails so far are the viewfinder's picture");
        // Other keys change nothing; Enter finishes, keeping them.
        key(&mut camera, Key::Right);
        assert_eq!(camera.mode, Mode::Trails);
        key(&mut camera, Key::Enter);
        run_until(&mut camera, 20_000, |c| c.saving.is_none());
        let note = camera.note.as_ref().map(|(n, _)| n.clone()).unwrap_or_default();
        assert!(note.starts_with("trails · ") && note.contains("frames over"), "{note}");
        let path = camera.last.as_ref().map(|(p, _)| p.path.clone()).unwrap();
        assert!(path.is_file() && photos::dimensions(&path).is_some());
    }

    #[test]
    fn a_time_lapse_takes_frames_on_its_beat_and_esc_keeps_them_as_a_video() {
        let _port = crate::smartcam::sim::exclusive(false);
        let mut camera = ready_in(Mode::Lapse);
        camera.lapse_every = 2;
        key(&mut camera, Key::Enter);
        assert!(camera.lapse.is_some());
        run_until(&mut camera, 8000, |c| c.lapse.as_ref().is_some_and(|t| t.saved >= 3));
        let work = camera.lapse.as_ref().map(|t| t.work.clone()).unwrap();
        assert!(lapse::frames_in(&work).len() >= 3, "each frame its own file meanwhile");
        assert!(camera.holds_dim(), "the screen dims while it runs");
        key(&mut camera, Key::Back);
        run_until(&mut camera, 15_000, |c| c.lapse.is_none() && c.note.as_ref().is_some_and(|(n, _)| n.starts_with("time-lapse kept")));
        let note = camera.note.as_ref().map(|(n, _)| n.clone()).unwrap_or_default();
        assert!(note.starts_with("time-lapse kept · "), "{note}");
        assert!(!work.exists(), "its frames' folder gone");
        let made = photos::list().into_iter().find(|p| lapse::is_lapse(&p.name)).expect("in the album");
        let avi = lapse::Avi::open(&made.path).unwrap();
        assert!(avi.frames.len() >= 3 && (avi.w, avi.h) == lapse::FRAME);
        assert!(photos::thumbnail(&made).is_some(), "its thumbnail, its first frame");
        std::fs::remove_file(&made.path).ok();
    }
}
