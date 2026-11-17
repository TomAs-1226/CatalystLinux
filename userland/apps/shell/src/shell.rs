//! The shell: a stack of screens, the transitions between them, the glass
//! island that carries link status and messages, and the status at the top.
//!
//! Motion follows Detent: every transition is a spring that starts from what
//! is on screen, input stays live throughout, and a transition can be turned
//! round or re-entered at any point without a jump. Enter and exit share a
//! path: an app grows out of the tile it was opened from and shrinks back
//! into it; a page inside an app slides in from the right and back out.

use bezel::canvas::{blend, sd_rrect};
use bezel::{Accent, Align, Canvas, Color, GlassShape, Rect, Tokens, Tone};
use detent::{role, Animated, Spring};

use crate::gfx::{Gfx, TextSize, H, PAD, TILE_RADIUS, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::screens::Home;

pub struct Settings {
    pub light: bool,
    pub calm: bool,
    pub text: TextSize,
    pub accent: Accent,
}

/// What a screen can reach while handling a key.
pub struct Cx<'a> {
    pub robot: &'a mut Robot,
    pub settings: &'a mut Settings,
    /// Set to show a message in the island.
    pub say: &'a mut Option<String>,
}

pub enum Nav {
    Stay,
    /// A page deeper in the same app: slides in from the right.
    Push(Box<dyn Screen>),
    /// An app, opened from the tile at this rectangle: grows out of it.
    Open(Box<dyn Screen>, Rect),
    Pop,
}

pub trait Screen {
    fn title(&self) -> &str;
    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav;
    fn draw(&mut self, g: &mut Gfx, robot: &Robot);
    /// True while a text field is being edited, so letters are not treated
    /// as navigation.
    fn wants_text(&self) -> bool {
        false
    }
    /// Advance the screen in time. True when it should be redrawn.
    fn tick(&mut self, _dt: f32) -> bool {
        false
    }
    /// True for a screen that should be looked at every frame beat even
    /// though nothing is moving, because something outside may change it at
    /// any moment (a shell printing). It is only redrawn when `tick` says so.
    fn busy(&self) -> bool {
        false
    }
    /// A word for the island when the screen opens.
    fn greeting(&self) -> Option<&'static str> {
        None
    }
}

const MESSAGE_SECONDS: f32 = 2.4;

/// How a screen arrived, which is also how it leaves.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Path {
    Slide,
    Grow(Rect),
}

impl Path {
    fn spring(&self) -> Spring {
        match self {
            // Bezel's page turn.
            Path::Slide => role::page(),
            // Bezel's app window: opening, closing and reopening.
            Path::Grow(_) => role::release(),
        }
    }
}

/// A finished frame of one screen, and the page dots it asked for.
type Picture = (Vec<u32>, Option<(usize, usize)>);

/// A screen on its way in or out. `progress` runs 0 (only the screen
/// underneath shows) to 1 (the arriving screen fills the panel). The two
/// screens are drawn once and then moved as pictures, so a frame of the
/// transition costs a copy rather than two redraws.
struct Transition {
    path: Path,
    progress: Animated,
    pictures: Option<[Picture; 2]>,
    /// The screen on top while it leaves: already off the stack, kept until
    /// it has gone (or is asked back).
    leaving: Option<Box<dyn Screen>>,
}

pub struct Shell {
    pub gfx: Gfx,
    pub robot: Robot,
    pub settings: Settings,
    /// Each screen with the path it arrived by.
    stack: Vec<(Box<dyn Screen>, Path)>,
    transition: Option<Transition>,
    island_w: Animated,
    /// Where the island's width was when its words last changed, for fading them in.
    island_from: f32,
    island_words: String,
    message: Option<(String, f32)>,
    dirty: bool,
    /// Whether anything was in motion at the last tick. Time only passes for
    /// motion that was already under way: an animation begun by a key after
    /// an idle spell starts from its beginning, not a wake-up interval in.
    moving: bool,
    /// The ground and its glows, drawn once per palette and copied each frame.
    background: Vec<u32>,
    background_for: Option<(u32, u32, u32)>,
    status: Status,
    /// The top screen as it was last drawn, and how deep the stack was then:
    /// a transition starts from this picture instead of drawing it again.
    last_screen: Picture,
    last_depth: usize,
}

/// The clock, battery and links at the top right, re-read now and then.
#[derive(Default)]
struct Status {
    clock: String,
    battery: Option<u32>,
    wired: bool,
    wireless: bool,
    age: f32,
}

impl Shell {
    pub fn new(robot: Robot) -> Self {
        Shell {
            gfx: Gfx::new(Tokens::dark()),
            robot,
            settings: Settings { light: false, calm: false, text: TextSize::Standard, accent: Accent::Orange },
            stack: vec![(Box::new(Home::new()), Path::Slide)],
            transition: None,
            island_w: Animated::new(60.0, role::release()).with_rest(0.2),
            island_from: 60.0,
            island_words: String::new(),
            message: None,
            dirty: true,
            moving: false,
            background: Vec::new(),
            background_for: None,
            status: Status { age: f32::MAX, ..Status::default() },
            last_screen: (Vec::new(), None),
            last_depth: usize::MAX,
        }
    }

    pub fn say(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), MESSAGE_SECONDS));
        self.dirty = true;
    }

    pub fn key(&mut self, key: Key) {
        self.dirty = true;
        let (top, _) = self.stack.last_mut().expect("the home screen is always there");
        // The Cardputer's arrows are Fn + F/X/Z/C. Outside text entry the bare
        // keys work too, so navigating never needs a modifier.
        let key = if top.wants_text() {
            key
        } else {
            match key {
                Key::Char('f') => Key::Up,
                Key::Char('x') => Key::Down,
                Key::Char('z') => Key::Left,
                Key::Char('c') => Key::Right,
                Key::Backspace => Key::Back,
                other => other,
            }
        };
        let mut say = None;
        let nav = top.key(key, &mut Cx { robot: &mut self.robot, settings: &mut self.settings, say: &mut say });
        if let Some(text) = say {
            self.say(text);
        }
        match nav {
            Nav::Stay => {}
            Nav::Push(screen) => self.open(screen, Path::Slide),
            Nav::Open(screen, from) => self.open(screen, Path::Grow(from)),
            Nav::Pop => self.close(),
        }
        self.apply_settings();
    }

    /// Bring the drawing in line with the settings: palette, calm, text size.
    pub fn apply_settings(&mut self) {
        self.dirty = true;
        self.gfx.accent = self.settings.accent;
        self.gfx.tokens = Tokens::of(self.settings.accent, if self.settings.light { Tone::Light } else { Tone::Dark });
        self.gfx.calm = self.settings.calm;
        self.gfx.set_text_size(self.settings.text);
    }

    fn open(&mut self, screen: Box<dyn Screen>, path: Path) {
        self.gfx.focus_reset();
        if let Some(words) = screen.greeting() {
            self.say(words);
        }
        // Asked back while it was leaving by the same path: it returns from
        // where it is, and the new copy is not needed.
        if let Some(t) = &mut self.transition {
            if t.path == path && t.leaving.is_some() {
                let returning = t.leaving.take().expect("checked above");
                self.stack.push((returning, path));
                t.progress.set(1.0);
                return;
            }
        }
        self.stack.push((screen, path));
        if self.settings.calm {
            self.transition = None;
            return;
        }
        let mut progress = Animated::new(0.0, path.spring()).with_rest(0.002);
        progress.set(1.0);
        self.transition = Some(Transition { path, progress, pictures: None, leaving: None });
    }

    fn close(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        self.gfx.focus_reset();
        let (screen, path) = self.stack.pop().expect("more than one screen");
        if self.settings.calm {
            self.transition = None;
            return;
        }
        match &mut self.transition {
            // Still arriving: turn it round where it is, keeping its speed.
            Some(t) if t.leaving.is_none() => {
                t.leaving = Some(screen);
                t.progress.set(0.0);
            }
            _ => {
                let mut progress = Animated::new(1.0, path.spring()).with_rest(0.002);
                progress.set(0.0);
                self.transition = Some(Transition { path, progress, pictures: None, leaving: Some(screen) });
            }
        }
    }

    /// Advance time. Returns whether a frame should be drawn.
    pub fn tick(&mut self, dt: f32) -> bool {
        // Motion only moves forward in time if it was already moving.
        let motion_dt = if self.moving { dt.min(0.05) } else { 0.0 };
        let mut moving = false;
        if let Some(t) = &mut self.transition {
            if t.progress.tick(motion_dt) || motion_dt == 0.0 {
                moving = true;
            } else {
                self.transition = None;
            }
        }
        moving |= self.island_w.tick(motion_dt);
        moving |= self.gfx.tick(motion_dt);
        if self.transition.is_none() {
            moving |= self.stack.last_mut().is_some_and(|(s, _)| s.tick(motion_dt));
        }
        if let Some((_, left)) = &mut self.message {
            *left -= dt;
            if *left <= 0.0 {
                self.message = None;
                self.dirty = true;
            }
        }
        // The clock and the battery, a few times a minute.
        self.status.age += dt;
        if self.status.age > 5.0 {
            self.status.age = 0.0;
            let clock = crate::sys::clock();
            let battery = crate::sys::battery_percent();
            let (wired, wireless) = (crate::sys::link_up("eth0"), crate::sys::link_up("wlan0"));
            if (clock.as_str(), battery, wired, wireless) != (self.status.clock.as_str(), self.status.battery, self.status.wired, self.status.wireless) {
                self.status = Status { clock, battery, wired, wireless, age: 0.0 };
                self.dirty = true;
            }
        }
        let changed = self.robot.refresh(dt);
        let draw = moving || changed || self.dirty;
        self.dirty = false;
        self.moving = moving;
        draw
    }

    /// Whether something on screen is in motion.
    pub fn moving(&self) -> bool {
        self.moving
    }

    /// Whether to keep to the frame beat: something is moving, or the
    /// screen in front wants looking at often.
    pub fn animating(&self) -> bool {
        self.moving || self.stack.last().is_some_and(|(s, _)| s.busy())
    }

    /// How far the transition in flight has come, if there is one.
    #[cfg(test)]
    fn progress(&self) -> Option<f32> {
        self.transition.as_ref().map(|t| t.progress.value())
    }

    pub fn draw(&mut self) {
        let g = &mut self.gfx;
        let t = g.tokens;
        let palette = (t.ground.to_u32(), t.signal.to_u32(), t.ice.to_u32());
        if self.background_for != Some(palette) {
            g.canvas.clear(t.ground);
            // The ground's glows: signal from the light's side, ice opposite.
            g.canvas.glow(40.0, 0.0, 150.0, t.signal, 0.07);
            g.canvas.glow(300.0, 170.0, 130.0, t.ice, 0.05);
            self.background = g.canvas.px.clone();
            self.background_for = Some(palette);
            if let Some(transition) = &mut self.transition {
                transition.pictures = None;
            }
        }
        g.canvas.px.copy_from_slice(&self.background);
        g.pager = None;
        // A title runs up to the island, however wide the island is just now.
        g.title_room = (W - self.island_w.value().max(g.island_h())) * 0.5 - PAD - 8.0;
        g.ox = 0.0;

        let depth = self.stack.len();
        if let Some(transition) = &mut self.transition {
            // Take the two pictures on the transition's first frame.
            if transition.pictures.is_none() {
                g.focus_static = true;
                let picture = |g: &mut Gfx, screen: &mut dyn Screen, home: bool| -> Picture {
                    g.canvas.px.copy_from_slice(&self.background);
                    g.pager = None;
                    Self::draw_screen(g, screen, &self.robot, home);
                    (g.canvas.px.clone(), g.pager.take().map(|(pages, current, _)| (pages, current)))
                };
                // The screen that was showing was drawn last frame. Keeping that
                // picture means a transition's first frame draws one screen,
                // not two, so it starts without a stall.
                let kept = |depth_then: usize| (self.last_depth == depth_then).then(|| self.last_screen.clone());
                transition.pictures = Some(match &mut transition.leaving {
                    Some(leaving) => {
                        let over = kept(depth + 1).unwrap_or_else(|| picture(g, leaving.as_mut(), false));
                        [picture(g, self.stack[depth - 1].0.as_mut(), depth == 1), over]
                    }
                    None => {
                        let (under, over) = self.stack.split_at_mut(depth - 1);
                        let under = kept(depth - 1).or_else(|| under.last_mut().map(|(s, _)| picture(g, s.as_mut(), depth == 2)));
                        [under.unwrap_or_else(|| (self.background.clone(), None)), picture(g, over[0].0.as_mut(), false)]
                    }
                });
                self.last_depth = usize::MAX;
                g.focus_static = false;
                g.canvas.px.copy_from_slice(&self.background);
            }
            let p = transition.progress.value();
            let [(under, under_dots), (over, over_dots)] = transition.pictures.as_ref().expect("taken above");
            match transition.path {
                Path::Slide => {
                    // The one underneath drifts a third as far as the one on top travels.
                    let (under_x, over_x) = ((-p * W * 0.3).round(), ((1.0 - p) * W).round());
                    blit_shifted(&mut g.canvas.px, under, W as usize, H as usize, under_x as i32);
                    blit_shifted(&mut g.canvas.px, over, W as usize, H as usize, over_x as i32);
                    g.pager = under_dots.map(|(pages, current)| (pages, current, under_x)).or(over_dots.map(|(pages, current)| (pages, current, over_x)));
                }
                Path::Grow(from) => {
                    g.canvas.px.copy_from_slice(under);
                    // From the tile's place and corners to the whole panel.
                    let grown = p.clamp(0.0, 1.0);
                    let window = from.lerp(&Rect::new(0.0, 0.0, W, H), grown);
                    blit_scaled(&mut g.canvas, over, window, TILE_RADIUS * (1.0 - grown), t.ground);
                    // The page dots belong to the screen underneath, until the window covers them.
                    if window.bottom() < H - 14.0 {
                        g.pager = under_dots.map(|(pages, current)| (pages, current, 0.0));
                    }
                }
            }
        } else {
            Self::draw_screen(g, self.stack[depth - 1].0.as_mut(), &self.robot, depth == 1);
            self.last_screen.0.clone_from(&g.canvas.px);
            self.last_screen.1 = g.pager.map(|(pages, current, _)| (pages, current));
            self.last_depth = depth;
        }
        g.ox = 0.0;

        // Status at the top right: links, battery, the time.
        let ty = g.ty;
        let bar_cy = 4.0 + g.island_h() * 0.5;
        let mut x = W - PAD - 2.0;
        x -= g.text_at(x, bar_cy, Align::Right, &self.status.clock, ty.caption, t.dim) + 7.0;
        if let Some(percent) = self.status.battery {
            let ink = if percent <= 15 { t.signal } else { t.dim };
            x -= g.text_at(x, bar_cy, Align::Right, &format!("{percent}%"), ty.caption, ink) + 7.0;
        }
        for (up, icon) in [(self.status.wireless, "wifi"), (self.status.wired, "lan")] {
            if up {
                g.icon(icon, false, x - 6.0, bar_cy, 13.0, t.dim);
                x -= 17.0;
            }
        }

        // The island: link status at rest, growing to carry a message.
        let linked = self.robot.linked();
        let (label, icon) = match &self.message {
            Some((text, _)) => (text.clone(), true),
            None if linked => (self.robot.string("Robot/Identity/Name").unwrap_or_else(|| "robot".into()).to_lowercase(), false),
            None => (self.robot.status(), false),
        };
        let caption = g.ty.caption;
        let island_h = g.island_h();
        // A message may take most of the width; status stays out of the title's way.
        let room = if icon { 250.0 } else { 96.0 };
        let label = g.text.fit(&label, &caption, room);
        let want = g.measure(&label, caption) + 34.0;
        if label != self.island_words {
            self.island_words = label.clone();
            self.island_from = self.island_w.value();
        }
        if (want - self.island_w.target()).abs() > 0.5 {
            // Growing arrives on the release spring; shrinking leaves on smooth.
            let growing = want > self.island_w.target();
            self.island_w.set_with(want, if growing { role::release() } else { role::smooth() });
            self.moving = true;
        }
        if self.settings.calm {
            self.island_w.jump(want);
        }
        let iw = self.island_w.value().max(island_h);
        let island = Rect::new((W - iw) * 0.5, 4.0, iw, island_h);
        // Every glass shape goes through one pass; each family is its own group.
        let mut shapes = vec![GlassShape::new(island, island_h * 0.5, 0)];
        let pager = g.pager.map(|(pages, current, ox)| {
            let w = pages as f32 * 9.0 + 7.0;
            (Rect::new((W - w) * 0.5 + ox, H - 13.0, w, 10.0), pages, current)
        });
        if let Some((platter, ..)) = pager {
            shapes.push(GlassShape::new(platter, 5.0, 1));
        }
        g.glass(&shapes);
        if let Some((platter, pages, current)) = pager {
            for i in 0..pages {
                let color = if i == current { t.ink } else { t.faint };
                g.canvas.circle(platter.x + 8.0 + i as f32 * 9.0, platter.cy(), 2.0, color, 1.0);
            }
        }

        // What sits on glass is not glass. New words fade in once the island
        // is past halfway (55%) to its new width.
        let travel = want - self.island_from;
        let grown = if travel.abs() < 1.0 { 1.0 } else { ((self.island_w.value() - self.island_from) / travel).clamp(0.0, 1.0) };
        let words = ((grown - 0.55) / 0.45).clamp(0.0, 1.0);
        g.canvas.set_clip(island.inset(2.0));
        let dot = if icon { t.signal } else if linked { t.ice } else { t.faint };
        g.canvas.circle(island.x + 12.0, island.cy(), 3.0, dot, 1.0);
        g.text_alpha(island.x + 22.0, island.cy(), &label, caption, t.ink, words);
        g.canvas.clear_clip();
        if g.take_motion_started() {
            self.moving = true;
        }
    }

    fn draw_screen(g: &mut Gfx, screen: &mut dyn Screen, robot: &Robot, _home: bool) {
        let t = g.tokens;
        let ty = g.ty;
        // The title shares the island's line; it gives way to a wide island.
        let bar_cy = 4.0 + g.island_h() * 0.5;
        let room = g.title_room;
        g.text_fit(PAD + 2.0, bar_cy, screen.title(), ty.title, t.ink, room);
        screen.draw(g, robot);
    }
}

/// Copy a full-screen picture into the frame, moved sideways by `dx` pixels.
fn blit_shifted(dst: &mut [u32], src: &[u32], w: usize, h: usize, dx: i32) {
    let shift = dx.unsigned_abs() as usize;
    if shift >= w {
        return;
    }
    let (to, from) = if dx >= 0 { (shift, 0) } else { (0, shift) };
    let n = w - shift;
    for y in 0..h {
        dst[y * w + to..y * w + to + n].copy_from_slice(&src[y * w + from..y * w + from + n]);
    }
}

/// Mix two packed pixels: `f` is 0 (all `a`) to 256 (all `b`). Integer only.
#[inline]
fn mix_px(a: u32, b: u32, f: u32) -> u32 {
    let rb = (((a & 0x00ff_00ff) * (256 - f) + (b & 0x00ff_00ff) * f) >> 8) & 0x00ff_00ff;
    let g = (((a & 0x0000_ff00) * (256 - f) + (b & 0x0000_ff00) * f) >> 8) & 0x0000_ff00;
    rb | g
}

/// Draw a full-screen picture scaled into `window`, with rounded corners, a
/// soft shadow that lifts it off what is underneath, and a hairline rim.
///
/// This runs every frame an app opens or closes, so it is written to cost
/// little: sampling is fixed-point, and the distance to the window's edge is
/// only worked out for pixels near that edge.
fn blit_scaled(canvas: &mut Canvas, src: &[u32], window: Rect, radius: f32, edge: Color) {
    let (w, h) = (canvas.w, canvas.h);
    if window.w < 1.0 || window.h < 1.0 {
        return;
    }
    // The whole panel: a straight copy.
    if window.w >= w as f32 - 0.5 && window.h >= h as f32 - 0.5 {
        canvas.px.copy_from_slice(src);
        return;
    }
    // Well inside the window, where no edge needs working out.
    let core = radius + 1.5;
    let inside = |x: f32, y: f32| {
        (x > window.x + 1.5 && x < window.right() - 1.5 && y > window.y + core && y < window.bottom() - core)
            || (x > window.x + core && x < window.right() - core && y > window.y + 1.5 && y < window.bottom() - 1.5)
    };

    // The shadow: a little lower than the window, as light from above casts it.
    const REACH: f32 = 8.0;
    let lifted = window.offset(0.0, 2.0);
    for y in ((window.y - REACH).floor() as i32).max(0)..((window.bottom() + REACH + 2.0).ceil() as i32).min(h) {
        for x in ((window.x - REACH).floor() as i32).max(0)..((window.right() + REACH).ceil() as i32).min(w) {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            if inside(fx, fy) {
                continue;
            }
            let d = sd_rrect(fx, fy, &lifted, radius);
            if d > 0.0 && d < REACH {
                let fall = 1.0 - d / REACH;
                let at = (y * w + x) as usize;
                canvas.px[at] = blend(canvas.px[at], Color::BLACK, 0.22 * fall * fall);
            }
        }
    }

    let x0 = (window.x.floor() as i32).max(0);
    let y0 = (window.y.floor() as i32).max(0);
    let x1 = (window.right().ceil() as i32).min(w);
    let y1 = (window.bottom().ceil() as i32).min(h);
    // Where each column of the window reads from, worked out once: the two
    // source columns and how far between them, in 256ths.
    let (sx, sy) = (w as f32 / window.w, h as f32 / window.h);
    let columns: Vec<(usize, usize, u32)> = (x0..x1)
        .map(|x| {
            let u = ((x as f32 + 0.5 - window.x) * sx - 0.5).clamp(0.0, (w - 1) as f32);
            let u0 = u as usize;
            (u0, (u0 + 1).min(w as usize - 1), (u.fract() * 256.0) as u32)
        })
        .collect();
    let edge_px = edge.to_u32();
    for y in y0..y1 {
        let v = ((y as f32 + 0.5 - window.y) * sy - 0.5).clamp(0.0, (h - 1) as f32);
        let v0 = v as usize;
        let (top, bottom) = (v0 * w as usize, (v0 + 1).min(h as usize - 1) * w as usize);
        let fv = (v.fract() * 256.0) as u32;
        let fy = y as f32 + 0.5;
        let row = (y * w) as usize;
        for (x, &(u0, u1, fu)) in (x0..x1).zip(&columns) {
            // Bilinear, so shrunken text stays text rather than breaking up.
            let px = mix_px(mix_px(src[top + u0], src[top + u1], fu), mix_px(src[bottom + u0], src[bottom + u1], fu), fv);
            let at = row + x as usize;
            let fx = x as f32 + 0.5;
            if inside(fx, fy) {
                canvas.px[at] = px;
                continue;
            }
            let d = sd_rrect(fx, fy, &window, radius);
            if d > 0.5 {
                continue;
            }
            // A one-pixel rim, so the window's edge reads against any ground.
            let rim = if d > -1.0 && radius > 0.5 { (90.0 * (d + 1.0).clamp(0.0, 1.0)) as u32 } else { 0 };
            let px = mix_px(px, edge_px, rim);
            let cover = ((0.5 - d).clamp(0.0, 1.0) * 256.0) as u32;
            canvas.px[at] = mix_px(canvas.px[at], px, cover);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f32 = 1.0 / 60.0;

    /// A shell on the demo robot, drawn once so Home knows where its tiles are.
    fn shell() -> Shell {
        let mut shell = Shell::new(Robot::demo());
        shell.apply_settings();
        for _ in 0..120 {
            shell.tick(FRAME);
            shell.draw();
        }
        shell
    }

    fn frames(shell: &mut Shell, n: usize, dt: f32) {
        for _ in 0..n {
            shell.tick(dt);
            shell.draw();
        }
    }

    #[test]
    fn a_key_after_an_idle_spell_starts_its_motion_from_the_beginning() {
        let mut shell = shell();
        // The device sleeps up to a quarter of a second between wake-ups.
        shell.tick(0.25);
        shell.key(Key::Enter);
        shell.tick(0.25);
        assert_eq!(shell.progress(), Some(0.0), "the first frame shows the start");
        shell.tick(FRAME);
        let p = shell.progress().unwrap();
        assert!(p > 0.0 && p < 0.1, "then one frame's worth of motion, not a wake-up's: {p}");
    }

    #[test]
    fn opening_can_be_turned_round_mid_flight_without_a_jump() {
        let mut shell = shell();
        shell.key(Key::Enter);
        frames(&mut shell, 9, FRAME);
        let before = shell.progress().unwrap();
        assert!(before > 0.2 && before < 0.9, "{before}");
        shell.key(Key::Back);
        assert_eq!(shell.progress(), Some(before), "it turns round where it is");
        shell.tick(FRAME);
        // It was moving outward, so it carries on a little before coming back.
        let just_after = shell.progress().unwrap();
        assert!((just_after - before).abs() < 0.12, "no jump: {before} then {just_after}");
        frames(&mut shell, 120, FRAME);
        assert_eq!(shell.progress(), None);
        assert_eq!(shell.stack.len(), 1, "back on Home");
    }

    #[test]
    fn a_closing_app_can_be_asked_back_from_where_it_is() {
        let mut shell = shell();
        shell.key(Key::Enter);
        frames(&mut shell, 120, FRAME);
        assert_eq!((shell.stack.len(), shell.progress()), (2, None));
        shell.key(Key::Back);
        frames(&mut shell, 8, FRAME);
        let closing = shell.progress().unwrap();
        assert!(closing > 0.1 && closing < 0.95, "{closing}");
        // Enter on the same tile: the same window returns, not a new one from the tile.
        shell.key(Key::Enter);
        assert_eq!(shell.stack.len(), 2);
        assert_eq!(shell.progress(), Some(closing));
        frames(&mut shell, 120, FRAME);
        assert_eq!((shell.stack.len(), shell.progress()), (2, None));
    }

    #[test]
    fn motion_does_not_depend_on_the_frame_rate() {
        // 0.3 s into opening an app at 60 frames a second and at 20.
        let at = |dt: f32| {
            let mut shell = shell();
            shell.key(Key::Enter);
            shell.tick(dt); // the starting frame
            frames(&mut shell, (0.3 / dt).round() as usize, dt);
            shell.progress().unwrap()
        };
        let (smooth, slow) = (at(1.0 / 60.0), at(1.0 / 20.0));
        assert!((smooth - slow).abs() < 0.005, "60 fps {smooth} against 20 fps {slow}");
    }

    #[test]
    fn the_calm_version_changes_screens_at_once() {
        let mut shell = shell();
        shell.settings.calm = true;
        shell.key(Key::Enter);
        assert_eq!((shell.stack.len(), shell.progress()), (2, None));
        shell.key(Key::Back);
        assert_eq!((shell.stack.len(), shell.progress()), (1, None));
    }
}
