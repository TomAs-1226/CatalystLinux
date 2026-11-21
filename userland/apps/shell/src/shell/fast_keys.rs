//! Keys faster than the motion: apps opened and closed before their windows
//! have landed, Esc pressed again and again, Ctrl-Tab mid-zoom, an app that
//! draws for itself started and quit at once. Every frame is filmed and read
//! for what it shows, so an old screen, a blank frame or the wrong app
//! showing for a moment fails, and so does a window that snaps instead of
//! moving on from where it was.

use super::*;
use crate::shell::tests::sandbox;

const FRAME: f32 = 1.0 / 60.0;

/// A test app: one colour over its whole page, so a frame can be read for
/// how much of it shows. Enter opens a page of a second colour over it.
struct Swatch {
    name: &'static str,
    rgb: u32,
    page: Option<(&'static str, u32)>,
}

impl Screen for Swatch {
    fn title(&self) -> &str {
        self.name
    }
    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Back => Nav::Pop,
            Key::Enter => match self.page {
                Some((name, rgb)) => Nav::Push(Box::new(Swatch { name, rgb, page: None })),
                None => Nav::Stay,
            },
            _ => Nav::Stay,
        }
    }
    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        g.canvas.rrect(Rect::new(0.0, 0.0, W, H), 0.0, Color::hex(self.rgb), 1.0);
    }
}

const A: u32 = 0xff00ff;
const B: u32 = 0x00ffff;
const P: u32 = 0xffff00;

/// App A, whose Enter opens page P; app B.
fn app_a() -> Box<dyn Screen> {
    Box::new(Swatch { name: "A", rgb: A, page: Some(("P", P)) })
}

fn app_b() -> Box<dyn Screen> {
    Box::new(Swatch { name: "B", rgb: B, page: None })
}

/// How much of a frame is `rgb`: each pixel counts as far as it is that
/// colour (a pixel half way between it and the tile beneath counts half).
fn share(px: &[u32], rgb: u32) -> f32 {
    let c = |v: u32, s: u32| ((v >> s) & 0xff) as f32;
    let sum: f32 = px
        .iter()
        .map(|&p| {
            let d = (c(p, 16) - c(rgb, 16)).abs().max((c(p, 8) - c(rgb, 8)).abs()).max((c(p, 0) - c(rgb, 0)).abs());
            (1.0 - d / 220.0).clamp(0.0, 1.0)
        })
        .sum();
    sum / px.len() as f32
}

/// A shell settled on Home, with no recent apps.
fn home() -> Shell {
    sandbox();
    let mut shell = Shell::new(Robot::demo());
    shell.apply_settings();
    for _ in 0..120 {
        shell.tick(FRAME);
        shell.draw();
    }
    shell
}

/// Two of Home's tiles, for the test apps to grow out of.
fn tiles(shell: &Shell) -> (Rect, Rect) {
    let first = shell.stack[0].0.spot().expect("Home's focused tile");
    (first, first.offset(first.w + 8.0, 0.0))
}

fn top(shell: &Shell) -> &str {
    shell.stack.last().unwrap().0.title()
}

/// Every frame from the one on the panel as the film starts, each read
/// for how much of a colour it shows.
struct Film {
    frames: Vec<Vec<u32>>,
}

impl Film {
    /// From what the panel shows now.
    fn start(shell: &Shell) -> Film {
        Film { frames: vec![shell.gfx.canvas.px.clone()] }
    }

    /// From a picture something else left on the panel.
    fn from(picture: Vec<u32>) -> Film {
        Film { frames: vec![picture] }
    }

    /// `n` frames more. None of them is blank: the bare wallpaper, with
    /// neither Home nor an app on it.
    #[track_caller]
    fn roll(&mut self, shell: &mut Shell, n: usize) {
        for _ in 0..n {
            shell.tick(FRAME);
            shell.draw();
            let frame = shell.gfx.canvas.px.clone();
            let bare = frame.iter().zip(&shell.background).filter(|(a, b)| a == b).count() as f32 / frame.len() as f32;
            assert!(bare < 0.85 || shell.switcher.is_some(), "frame {} is blank: {bare:.2} of it the bare wallpaper", self.frames.len());
            self.frames.push(frame);
        }
    }

    fn shares(&self, rgb: u32) -> Vec<f32> {
        self.frames.iter().map(|f| share(f, rgb)).collect()
    }

    fn last(&self, rgb: u32) -> f32 {
        share(self.frames.last().unwrap(), rgb)
    }

    /// `rgb` moves on from frame to frame: no frame in which much more or
    /// much less of it shows than in the one before.
    #[track_caller]
    fn smooth(&self, rgb: u32, what: &str) {
        let s = self.shares(rgb);
        let (step, at) = s.windows(2).enumerate().map(|(i, w)| ((w[1] - w[0]).abs(), i + 1)).fold((0.0, 0), |a, b| if b.0 > a.0 { b } else { a });
        let around: Vec<String> = s.iter().skip(at.saturating_sub(3)).take(6).map(|s| format!("{s:.2}")).collect();
        assert!(step < STEP, "{what}: {rgb:06x} jumps by {step:.2} at frame {at} ({})", around.join(" "));
    }

    /// The most of the panel that changes visibly from one frame to the
    /// next, and where.
    fn largest_change(&self) -> (f32, usize) {
        let visibly = |a: u32, b: u32| (0..3).any(|i| ((a >> (8 * i)) & 0xff).abs_diff((b >> (8 * i)) & 0xff) > 24);
        self.frames
            .windows(2)
            .enumerate()
            .map(|(i, w)| (w[0].iter().zip(&w[1]).filter(|(a, b)| visibly(**a, **b)).count() as f32 / w[0].len() as f32, i + 1))
            .fold((0.0, 0), |a, b| if b.0 > a.0 { b } else { a })
    }

    /// `rgb` does not show from frame `from` on.
    #[track_caller]
    fn gone_from(&self, rgb: u32, from: usize, what: &str) {
        for (i, s) in self.shares(rgb).iter().enumerate().skip(from) {
            assert!(*s < GONE, "{what}: {rgb:06x} still shows at frame {i} ({s:.2})");
        }
    }
}

/// The most a window may change between frames at 60 a second: an app
/// growing from its tile untroubled changes by under half of this.
const STEP: f32 = 0.3;
/// Home's own colours read as this much of any test colour, at most.
const GONE: f32 = 0.04;

#[test]
fn an_untroubled_open_and_close_moves_in_small_steps() {
    let mut shell = home();
    let (tile, _) = tiles(&shell);
    let mut film = Film::start(&shell);
    shell.follow(Nav::Open(app_a(), tile));
    film.roll(&mut shell, 60);
    assert!(film.last(A) > 0.9);
    shell.key(Key::Back);
    film.roll(&mut shell, 60);
    let s = film.shares(A);
    let step = s.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
    assert!(step < STEP * 0.5, "{step}");
    film.gone_from(A, 100, "closed");
}

#[test]
fn opened_and_closed_at_once_the_window_turns_round_where_it_is() {
    for k in [0, 1, 2, 3, 5, 8, 12] {
        let mut shell = home();
        let (tile, _) = tiles(&shell);
        let mut film = Film::start(&shell);
        shell.follow(Nav::Open(app_a(), tile));
        film.roll(&mut shell, k);
        shell.key(Key::Back);
        film.roll(&mut shell, 60);
        film.smooth(A, &format!("closed {k} frames in"));
        film.gone_from(A, k + 40, "closed");
        assert_eq!(shell.stack.len(), 1);
    }
}

#[test]
fn closed_and_reopened_at_once_the_same_window_comes_back() {
    for k in [0, 1, 2, 4, 8] {
        let mut shell = home();
        let (tile, _) = tiles(&shell);
        shell.follow(Nav::Open(app_a(), tile));
        Film::start(&shell).roll(&mut shell, 60);
        let mut film = Film::start(&shell);
        shell.key(Key::Back);
        film.roll(&mut shell, k);
        shell.follow(Nav::Open(app_a(), tile));
        film.roll(&mut shell, 60);
        film.smooth(A, &format!("reopened {k} frames in"));
        assert!(film.last(A) > 0.9, "{k}: back in front");
        assert_eq!((shell.stack.len(), top(&shell)), (2, "A"));
    }
    // Back, Enter, Back, Enter, Back… a few frames apart: it goes back and
    // forth from wherever it is, and is on Home, kept, at the end.
    for k in [0, 2, 5] {
        let mut shell = home();
        let (tile, _) = tiles(&shell);
        shell.follow(Nav::Open(app_a(), tile));
        Film::start(&shell).roll(&mut shell, 60);
        let mut film = Film::start(&shell);
        for _ in 0..3 {
            shell.key(Key::Back);
            film.roll(&mut shell, k);
            shell.follow(Nav::Open(app_a(), tile));
            film.roll(&mut shell, k);
        }
        shell.key(Key::Back);
        film.roll(&mut shell, 60);
        film.smooth(A, &format!("back and forth every {k} frames"));
        film.gone_from(A, film.frames.len() - 5, "on Home");
        assert_eq!(shell.stack.len(), 1);
        assert!(shell.recents.find("A").is_some());
    }
}

#[test]
fn another_app_opened_while_one_closes_lets_it_finish_going() {
    for k in [0, 1, 2, 4, 8, 12] {
        let mut shell = home();
        let (tile, other) = tiles(&shell);
        shell.follow(Nav::Open(app_a(), tile));
        Film::start(&shell).roll(&mut shell, 60);
        let mut film = Film::start(&shell);
        shell.key(Key::Back);
        film.roll(&mut shell, k);
        shell.follow(Nav::Open(app_b(), other));
        film.roll(&mut shell, 60);
        film.smooth(A, &format!("A closing, B opened {k} frames in"));
        film.smooth(B, &format!("B opening {k} frames into A's close"));
        film.gone_from(A, k + 45, "A has gone into its tile");
        assert!(film.last(B) > 0.9);
        assert_eq!(top(&shell), "B");
        assert!(shell.recents.find("A").is_some(), "{k}: A is kept among the recent apps");
    }
}

#[test]
fn esc_pressed_again_and_again_never_snaps() {
    // An app with a page open: Esc, Esc, Esc… a few frames apart.
    for k in [0, 1, 2, 4, 8] {
        let mut shell = home();
        let (tile, _) = tiles(&shell);
        shell.follow(Nav::Open(app_a(), tile));
        Film::start(&shell).roll(&mut shell, 60);
        shell.key(Key::Enter);
        Film::start(&shell).roll(&mut shell, 60);
        assert_eq!(top(&shell), "P");
        let mut film = Film::start(&shell);
        for _ in 0..5 {
            shell.key(Key::Back);
            film.roll(&mut shell, k);
        }
        film.roll(&mut shell, 60);
        film.smooth(P, &format!("esc every {k} frames: the page"));
        film.smooth(A, &format!("esc every {k} frames: the app"));
        film.gone_from(A, film.frames.len() - 10, "back on Home");
        assert_eq!(shell.stack.len(), 1);
        assert!(shell.recents.find("A").is_some());
    }
    // Opening, then Esc pressed over and over.
    let mut shell = home();
    let (tile, _) = tiles(&shell);
    let mut film = Film::start(&shell);
    shell.follow(Nav::Open(app_a(), tile));
    film.roll(&mut shell, 3);
    for _ in 0..6 {
        shell.key(Key::Back);
        film.roll(&mut shell, 1);
    }
    film.roll(&mut shell, 60);
    film.smooth(A, "esc over and over as it opens");
    assert_eq!(shell.stack.len(), 1);
}

#[test]
fn a_page_opened_while_its_app_still_grows_lets_the_app_finish_growing() {
    // (Opened before the app was ever drawn, the page grows out of the tile with it.)
    for k in [0, 1, 3, 6, 10] {
        let mut shell = home();
        let (tile, _) = tiles(&shell);
        let mut film = Film::start(&shell);
        shell.follow(Nav::Open(app_a(), tile));
        film.roll(&mut shell, k);
        shell.key(Key::Enter);
        film.roll(&mut shell, 60);
        film.smooth(A, &format!("a page opened {k} frames into the app's opening"));
        film.smooth(P, "the page");
        assert!(film.last(P) > 0.9);
    }
}

#[test]
fn enter_then_esc_esc_as_an_app_opens_turns_the_app_round_from_where_it_is() {
    // An app opened, a page of it at once, and straight back out: the app's
    // window turns round from wherever its opening had got to, not from whole.
    for (k, j, m) in [(2, 1, 1), (3, 2, 0), (5, 3, 2), (1, 0, 0), (4, 6, 3)] {
        let mut shell = home();
        let (tile, _) = tiles(&shell);
        let mut film = Film::start(&shell);
        shell.follow(Nav::Open(app_a(), tile));
        film.roll(&mut shell, k);
        shell.key(Key::Enter);
        film.roll(&mut shell, j);
        shell.key(Key::Back);
        film.roll(&mut shell, m);
        shell.key(Key::Back);
        film.roll(&mut shell, 60);
        let what = format!("open, {k} frames, enter, {j}, esc, {m}, esc");
        film.smooth(A, &what);
        film.smooth(P, &what);
        film.gone_from(A, film.frames.len() - 5, "on Home");
        assert_eq!(shell.stack.len(), 1);
    }
}

#[test]
fn ctrl_tab_mid_zoom_neither_snaps_the_window_nor_freezes_it() {
    // Opening: the window keeps growing behind the cards, and is there when they go.
    for (k, j) in [(1, 30), (3, 2), (5, 6), (8, 30), (12, 1)] {
        let mut shell = home();
        let (tile, _) = tiles(&shell);
        let mut film = Film::start(&shell);
        shell.follow(Nav::Open(app_a(), tile));
        film.roll(&mut shell, k);
        shell.key(Key::Ctrl('\t'));
        film.roll(&mut shell, j);
        shell.key(Key::Back);
        film.roll(&mut shell, 60);
        film.smooth(A, &format!("ctrl-tab {k} frames into opening, esc {j} later"));
        assert!(film.last(A) > 0.9, "{k}, {j}: the app is in front");
        assert_eq!(top(&shell), "A");
    }
    // Closing: the window goes on into its tile behind the cards, and the app is a card.
    for (k, j) in [(1, 30), (3, 2), (5, 6), (8, 30)] {
        let mut shell = home();
        let (tile, _) = tiles(&shell);
        shell.follow(Nav::Open(app_a(), tile));
        Film::start(&shell).roll(&mut shell, 60);
        let mut film = Film::start(&shell);
        shell.key(Key::Back);
        film.roll(&mut shell, k);
        shell.key(Key::Ctrl('\t'));
        assert_eq!(shell.switcher.as_ref().unwrap().cards.first().map(|c| c.title.as_str()), Some("A"), "the app just left is the first card");
        film.roll(&mut shell, j);
        shell.key(Key::Back);
        film.roll(&mut shell, 60);
        film.smooth(A, &format!("ctrl-tab {k} frames into closing, esc {j} later"));
        film.gone_from(A, film.frames.len() - 5, "on Home");
        assert_eq!(shell.stack.len(), 1);
    }
}

#[test]
fn the_card_of_an_app_still_opening_shows_the_app() {
    let mut shell = home();
    let (tile, _) = tiles(&shell);
    shell.follow(Nav::Open(app_a(), tile));
    Film::start(&shell).roll(&mut shell, 3);
    shell.key(Key::Ctrl('\t'));
    let card = &shell.switcher.as_ref().unwrap().cards[0];
    assert!(card.current && card.title == "A");
    assert!(share(&card.thumb, A) > 0.8, "not blank: {}", share(&card.thumb, A));
    // The same with the keys in one batch, before a frame was drawn.
    let mut shell = home();
    shell.follow(Nav::Open(app_a(), tile));
    shell.key(Key::Ctrl('\t'));
    assert!(share(&shell.switcher.as_ref().unwrap().cards[0].thumb, A) > 0.8);
}

#[test]
fn esc_while_the_recent_apps_go_lands_on_the_app_without_a_snap() {
    for k in [1, 3, 6] {
        let mut shell = home();
        let (tile, _) = tiles(&shell);
        shell.follow(Nav::Open(app_a(), tile));
        Film::start(&shell).roll(&mut shell, 60);
        let mut film = Film::start(&shell);
        shell.key(Key::Ctrl('\t'));
        film.roll(&mut shell, 20);
        shell.key(Key::Back);
        film.roll(&mut shell, k);
        // The cards are still going: this Esc is the app's, and closes it.
        shell.key(Key::Back);
        film.roll(&mut shell, 60);
        film.smooth(A, &format!("esc {k} frames after the cards began to go"));
        film.gone_from(A, film.frames.len() - 5, "on Home");
        assert_eq!(shell.stack.len(), 1);
    }
}

#[test]
fn a_window_closing_shows_its_own_app_however_fast_the_keys() {
    // B among the recent apps, A in front; then in one go, before a frame:
    // Ctrl-Tab, Enter (B comes back) and Esc (B closes again).
    let mut shell = home();
    let (tile, other) = tiles(&shell);
    shell.follow(Nav::Open(app_b(), other));
    Film::start(&shell).roll(&mut shell, 60);
    shell.key(Key::Back);
    Film::start(&shell).roll(&mut shell, 60);
    shell.follow(Nav::Open(app_a(), tile));
    Film::start(&shell).roll(&mut shell, 60);
    shell.key(Key::Ctrl('\t'));
    shell.key(Key::Enter);
    shell.key(Key::Back);
    let mut film = Film::start(&shell);
    film.roll(&mut shell, 60);
    // A was on the panel: it fades into Home, rather than vanishing in a
    // frame. B's picture is B's own, on its card afterwards too.
    film.smooth(A, "what showed as the keys came");
    film.gone_from(A, 40, "on Home");
    assert_eq!(shell.stack.len(), 1);
    let card = &shell.recents.iter().find(|r| r.title() == "B").expect("B kept").thumb;
    assert!(share(card, A) < GONE, "B's card shows A: {}", share(card, A));
}

#[test]
fn an_app_opened_from_its_card_and_closed_at_once_leaves_no_picture_of_the_cards() {
    for k in [2, 5, 9] {
        // B among the recent apps, A in front: Ctrl-Tab, Enter on B's card,
        // and Esc while B still grows out of it.
        let mut shell = home();
        let (tile, other) = tiles(&shell);
        shell.follow(Nav::Open(app_b(), other));
        Film::start(&shell).roll(&mut shell, 60);
        shell.key(Key::Back);
        Film::start(&shell).roll(&mut shell, 60);
        shell.follow(Nav::Open(app_a(), tile));
        Film::start(&shell).roll(&mut shell, 60);
        shell.key(Key::Ctrl('\t'));
        Film::start(&shell).roll(&mut shell, 30);
        let mut film = Film::start(&shell);
        shell.key(Key::Enter);
        film.roll(&mut shell, k);
        shell.key(Key::Back);
        film.roll(&mut shell, 90);
        film.smooth(B, &format!("esc {k} frames in"));
        // Back in its card, the cards as they showed give way to Home: they
        // fade, and are not swapped for it in one frame (over a third of the
        // panel at once, before).
        let (change, at) = film.largest_change();
        assert!(change < 0.2, "{k}: {change:.2} of the panel changes at once, at frame {at}");
        assert_eq!(shell.stack.len(), 1);
    }
}

#[test]
fn esc_while_an_app_brought_back_pages_deep_still_grows_slides_its_page_away() {
    for k in [0, 2, 5, 9] {
        // A kept with its page open (left for B from the recent apps).
        let mut shell = home();
        let (tile, other) = tiles(&shell);
        shell.follow(Nav::Open(app_b(), other));
        Film::start(&shell).roll(&mut shell, 60);
        shell.key(Key::Back);
        Film::start(&shell).roll(&mut shell, 60);
        shell.follow(Nav::Open(app_a(), tile));
        Film::start(&shell).roll(&mut shell, 60);
        shell.key(Key::Enter);
        Film::start(&shell).roll(&mut shell, 60);
        shell.key(Key::Ctrl('\t'));
        Film::start(&shell).roll(&mut shell, 30);
        shell.key(Key::Enter);
        Film::start(&shell).roll(&mut shell, 60);
        assert_eq!(top(&shell), "B");
        shell.key(Key::Back);
        Film::start(&shell).roll(&mut shell, 60);
        // Opened again from its tile, it grows back showing the page.
        let mut film = Film::start(&shell);
        shell.follow(Nav::Open(app_a(), tile));
        assert_eq!((shell.stack.len(), top(&shell)), (3, "P"), "back as it was left");
        film.roll(&mut shell, k);
        // Esc is the page's: it goes, and the app goes on growing beneath it.
        shell.key(Key::Back);
        film.roll(&mut shell, 60);
        film.smooth(P, &format!("esc {k} frames in: the page"));
        film.smooth(A, &format!("esc {k} frames in: the app"));
        assert!(film.last(A) > 0.9, "{k}: the app in front, whole");
        assert_eq!((shell.stack.len(), top(&shell)), (2, "A"));
    }
}

#[test]
fn any_fast_run_of_open_close_page_and_ctrl_tab_moves_without_a_jump() {
    // A small, fixed generator: the same runs every time, so a failure can
    // be replayed from the keys it names. (Run 1500 times, from this seed
    // and 0x2545F4914F6CDD1D, it found several of the cases above.)
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    for run in 0..120 {
        let mut shell = home();
        let (tile, other) = tiles(&shell);
        let mut film = Film::start(&shell);
        let mut said = Vec::new();
        for _ in 0..14 {
            let on_home = shell.stack.len() == 1 && shell.switcher.is_none();
            let key = match next(6) {
                0 if on_home => {
                    shell.follow(Nav::Open(app_a(), tile));
                    "open-a"
                }
                1 if on_home => {
                    shell.follow(Nav::Open(app_b(), other));
                    "open-b"
                }
                2 | 3 => {
                    shell.key(Key::Back);
                    "esc"
                }
                4 if top(&shell) == "A" || shell.switcher.is_some() => {
                    shell.key(Key::Enter);
                    "enter"
                }
                5 => {
                    shell.key(Key::Ctrl('\t'));
                    "ctrl-tab"
                }
                _ => continue,
            };
            let wait = [0, 1, 2, 3, 5, 8][next(6)];
            said.push(format!("{key}@{} {wait}", film.frames.len()));
            film.roll(&mut shell, wait);
        }
        film.roll(&mut shell, 90);
        let what = format!("run {run} ({})", said.join(" "));
        for rgb in [A, B, P] {
            film.smooth(rgb, &what);
        }
    }
}

#[test]
fn a_real_app_asked_back_at_once_shows_its_page_not_an_empty_one() {
    // Robot from Home's first tile, Esc a frame or two in, Enter a moment
    // later: its rows go on rising in its window whichever way it goes, and
    // it lands showing them, with no empty page and no pop at the end.
    for (k, j) in [(1, 2), (2, 1), (4, 6)] {
        let mut shell = home();
        shell.key(Key::Enter);
        while shell.transition.is_none() {
            Film::start(&shell).roll(&mut shell, 1);
        }
        let mut film = Film::start(&shell);
        film.roll(&mut shell, k);
        shell.key(Key::Back);
        film.roll(&mut shell, j);
        shell.key(Key::Enter);
        film.roll(&mut shell, 90);
        assert_eq!(top(&shell), "Robot");
        let (change, at) = film.largest_change();
        assert!(change < 0.25, "{k}, {j}: {change:.2} of the panel changes at once, at frame {at}");
    }
}

// ------------------------------------------------- an app that draws itself ---

fn paint() -> crate::apps::App {
    crate::apps::App {
        name: "Paint".into(),
        icon: "apps".into(),
        about: String::new(),
        run: "paint".into(),
        dir: std::path::PathBuf::new(),
        kind: crate::apps::Kind::Own,
    }
}

/// Paint opened from its tile, landed, and handed the panel.
fn painting(shell: &mut Shell, tile: Rect) {
    shell.follow(Nav::Open(paint().open(), tile));
    Film::start(shell).roll(shell, 60);
    assert_eq!(shell.take_launch().map(|app| app.name), Some("Paint".into()), "started once its page has landed");
}

#[test]
fn an_app_that_draws_for_itself_starts_every_time_it_is_opened() {
    let mut shell = home();
    let (tile, _) = tiles(&shell);
    painting(&mut shell, tile);
    shell.launch_ended(Ok(()), None);
    assert!(shell.take_launch().is_none(), "its page is closing");
    Film::start(&shell).roll(&mut shell, 60);
    assert_eq!(shell.stack.len(), 1);
    // Opened again: not a page left over from last time that never starts it.
    painting(&mut shell, tile);
    shell.launch_ended(Ok(()), None);
    // Asked back while its window is still closing: it lands and starts again.
    Film::start(&shell).roll(&mut shell, 4);
    shell.follow(Nav::Open(paint().open(), tile));
    Film::start(&shell).roll(&mut shell, 60);
    assert_eq!(shell.take_launch().map(|app| app.name), Some("Paint".into()));
    // Esc while it is still growing: it goes back, and is not kept to come back to.
    shell.launch_ended(Ok(()), None);
    Film::start(&shell).roll(&mut shell, 60);
    shell.follow(Nav::Open(paint().open(), tile));
    Film::start(&shell).roll(&mut shell, 3);
    shell.key(Key::Back);
    Film::start(&shell).roll(&mut shell, 60);
    assert!(shell.take_launch().is_none());
    assert!(shell.recents.find("Paint").is_none());
    painting(&mut shell, tile);
}

#[test]
fn an_app_that_draws_for_itself_waits_for_its_page_to_show_alone() {
    // The recent apps came up over it as it grew: not under them.
    let mut shell = home();
    let (tile, _) = tiles(&shell);
    shell.follow(Nav::Open(paint().open(), tile));
    Film::start(&shell).roll(&mut shell, 3);
    shell.key(Key::Ctrl('\t'));
    Film::start(&shell).roll(&mut shell, 60);
    assert!(shell.take_launch().is_none(), "the cards are up");
    shell.key(Key::Back);
    Film::start(&shell).roll(&mut shell, 60);
    assert!(shell.take_launch().is_some(), "they have gone");
    // Nor under the control centre.
    let mut shell = home();
    shell.follow(Nav::Open(paint().open(), tile));
    Film::start(&shell).roll(&mut shell, 3);
    shell.key(Key::Ctrl(' '));
    Film::start(&shell).roll(&mut shell, 60);
    assert!(shell.take_launch().is_none(), "the control centre is down");
    shell.key(Key::Back);
    Film::start(&shell).roll(&mut shell, 60);
    assert!(shell.take_launch().is_some());
}

#[test]
fn an_app_that_draws_for_itself_closes_from_its_own_last_picture() {
    let mut shell = home();
    let (tile, _) = tiles(&shell);
    painting(&mut shell, tile);
    // It left the panel all one colour.
    let left = vec![A; (W * H) as usize];
    shell.launch_ended(Ok(()), Some(left.clone()));
    assert!(shell.animating(), "the shell draws again at once, not after an idle wait");
    let mut film = Film::from(left);
    film.roll(&mut shell, 60);
    film.smooth(A, "its picture going back into the tile");
    film.gone_from(A, 50, "on Home");
    assert_eq!(shell.stack.len(), 1);
}
