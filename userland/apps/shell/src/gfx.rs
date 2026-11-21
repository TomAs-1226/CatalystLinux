//! What screens draw with: Bezel's surfaces, type and icons on the 320 × 170
//! panel, plus the few controls every screen shares.
//!
//! The panel is 1.9 inches, about 190 pixels per inch, so type cannot be
//! Bezel's 720-panel sizes scaled down (a 15 px label would be 6.7 px, under
//! a millimetre). Shapes scale by 320 / 720; type has its own scale with a
//! floor, chosen in System, and every layout is measured from it.

use bezel::{Align, Canvas, Color, Glass, GlassShape, Icons, Rect, Text, TextStyle, Tokens};
use detent::{role, Animated, Spring};

pub const W: f32 = 320.0;
pub const H: f32 = 170.0;
/// Page padding and the gap between tiles (Bezel's 30 and 14, scaled).
pub const PAD: f32 = 8.0;
pub const GAP: f32 = 5.0;
/// Tile corner radius (Bezel's 32, scaled).
pub const TILE_RADIUS: f32 = 14.0;

/// Pages rising in: an arriving page's rows come up this far into place,
/// one after another this far apart, all of them within `REVEAL_OVER`.
/// (Our own tuning: Bezel's launcher cascade, carried to every page.)
const RISE: f32 = 10.0;
const STAGGER: f32 = 0.035;
/// The ninth block and after arrive together; the last has settled by then.
pub const RISE_LAST: f32 = 8.0;
const REVEAL_OVER: f32 = RISE_LAST * STAGGER + 0.55;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextSize {
    Compact,
    Standard,
    Large,
}

impl TextSize {
    pub fn name(self) -> &'static str {
        match self {
            TextSize::Compact => "compact",
            TextSize::Standard => "standard",
            TextSize::Large => "large",
        }
    }
    pub fn next(self) -> Self {
        match self {
            TextSize::Compact => TextSize::Standard,
            TextSize::Standard => TextSize::Large,
            TextSize::Large => TextSize::Compact,
        }
    }
    pub fn from_name(name: &str) -> Option<Self> {
        [TextSize::Compact, TextSize::Standard, TextSize::Large].into_iter().find(|s| s.name() == name)
    }
}

/// The type roles at one text size.
#[derive(Clone, Copy)]
pub struct Type {
    /// The smallest text: lowercase mono captions and hints.
    pub caption: TextStyle,
    /// Mono values.
    pub label: TextStyle,
    /// Rows and control labels.
    pub body: TextStyle,
    /// Names of things.
    pub name: TextStyle,
    /// Screen titles.
    pub title: TextStyle,
    /// Multiplies display (glanceable value) sizes.
    pub display_scale: f32,
}

impl Type {
    pub fn of(size: TextSize) -> Type {
        let (caption, label, body, name, title, display_scale) = match size {
            TextSize::Compact => (10.0, 11.0, 12.0, 13.0, 15.0, 0.9),
            // The default: nothing under 11 px (about 1.5 mm on this panel),
            // and no larger than the screen can carry without cutting words.
            TextSize::Standard => (11.0, 12.0, 13.0, 15.0, 16.0, 1.0),
            TextSize::Large => (13.0, 14.0, 15.0, 17.0, 18.0, 1.1),
        };
        Type {
            caption: TextStyle::label(caption),
            label: TextStyle::label(label),
            body: TextStyle::body(body),
            name: TextStyle::name(name),
            title: TextStyle::title(title),
            display_scale,
        }
    }

    /// The display voice, for a glanceable value.
    pub fn display(&self, size: f32) -> TextStyle {
        TextStyle::display(size * self.display_scale)
    }
}

/// Where a line's baseline goes to sit centred on `cy`.
fn baseline_for(style: &TextStyle, cy: f32) -> f32 {
    (cy + style.size * 0.36).round()
}

pub struct Gfx {
    pub canvas: Canvas,
    pub text: Text,
    pub icons: Icons,
    /// The objects rendered beforehand in three dimensions (`object3d`).
    pub objects: crate::object3d::Objects,
    /// Icon names asked for that were never built in: drawn as nothing, so
    /// kept to be caught (the gallery and the tests fail on any).
    missing_icons: std::collections::BTreeSet<String>,
    pub glass: Glass,
    pub tokens: Tokens,
    /// The accent the palette was grown from.
    pub accent: bezel::Accent,
    pub ty: Type,
    pub text_size: TextSize,
    /// Horizontal offset of what is being drawn: screens slide during a page turn.
    pub ox: f32,
    /// Reduced transparency and motion.
    pub calm: bool,
    /// Which Home screen is in use.
    pub home: crate::shell::HomeStyle,
    /// Whether a robot linking opens the Robot app from Home.
    pub open_robot: bool,
    /// Whether a match's summary opens once its log is pulled.
    pub match_report: bool,
    /// The glance page's cards, and the value pinned to it.
    pub glance: Vec<crate::shell::Widget>,
    pub pinned: Option<String>,
    /// Home's arrangement (apps in order, apps hidden), and the clock's
    /// settings (24-hour, seconds).
    pub arrangement: crate::shell::Arrangement,
    pub clock: (bool, bool),
    /// What the island shows at rest, and whether Home is greeted.
    pub island: crate::shell::IslandRest,
    pub greeting: bool,
    /// Developer options: on or not, the animation scale, and Show key presses.
    pub developer: bool,
    pub motion_scale: f32,
    pub show_keys: bool,
    /// Standby: seconds with no key before the screen goes off (0 never),
    /// and the processor's speed choice.
    pub screen_off: u32,
    pub power: crate::standby::Power,
    /// Settings › Battery's choices, for the rows that show them.
    pub battery: crate::power::Choices,
    /// Lift to wake and face down to sleep.
    pub lift_wake: bool,
    pub face_down: bool,
    /// The lock screen's and StandBy's choices, for their Settings pages.
    pub lock: crate::lock::Choices,
    /// The look chosen, its theme on its own for brevity, what every corner
    /// is multiplied by, and tiles across Home.
    pub look: crate::look::Look,
    pub theme: crate::look::Theme,
    corner: f32,
    pub columns: usize,
    /// Page dots a screen wants on the glass pager: (pages, current, x offset).
    pub pager: Option<(usize, usize, f32)>,
    /// The width a screen title may take before it meets the island.
    pub title_room: f32,
    /// The focus ring, which glides to whatever is focused.
    focus: Focus,
    /// Where the ring was last drawn (its stroke's outer rectangle's middle
    /// line), for looking at its motion.
    pub(crate) last_ring: Rect,
    /// Draw the ring where it is asked for and leave the glide alone: set
    /// while a screen is drawn as a picture for a transition.
    pub focus_static: bool,
    /// Set when drawing gave something a new place to move to.
    motion_started: bool,
    /// Pages rising in: seconds since the page in front began to arrive,
    /// while its rows are still rising.
    reveal: Option<f32>,
    /// Enter landing: a state layer and a small squash on what it pressed,
    /// at 1 as the key lands and springing back to 0.
    press: Animated,
    /// The GPU, when it draws the glass and growing windows. Wanted unless
    /// Settings › Display › Graphics says the processor; if the kernel's
    /// driver has not loaded yet when the shell starts, it is tried again
    /// once its device appears.
    pub gpu: Option<crate::gpu::Gpu>,
    gpu_wanted: Option<bool>,
    gpu_awaiting_device: bool,
    /// The GPU being opened, on a thread of its own.
    gpu_opening: Option<std::sync::mpsc::Receiver<Result<crate::gpu::Gpu, String>>>,
    /// How far the list on screen is scrolled, in rows. It glides to where
    /// the list was moved rather than jumping a row at a time.
    scroll: Animated,
    scroll_placed: bool,
    /// While a list is mid-glide its rows are not yet where they will rest:
    /// this is how far off they are, so the focus ring can head for the
    /// focused row's resting place instead of chasing it.
    ring_shift: f32,
}

/// The focus ring's place, its size and its corners. It glides to whatever is
/// focused and keeps its speed when the focus moves on again before it lands
/// (a key held, keys pressed quickly), so it never starts over.
struct Focus {
    x: Animated,
    y: Animated,
    w: Animated,
    h: Animated,
    radius: Animated,
    /// False until the ring has been put somewhere on this screen: its first
    /// place is taken at once, with nothing to glide from.
    placed: bool,
    /// How far what the ring sits on was moved sideways when it was last
    /// drawn (see [`Gfx::focus_ring_on`]), and whether it was on something
    /// that moves so (a strip of pages) rather than on the screen itself.
    along: f32,
    riding: bool,
}

/// Material 3 Expressive's spatial springs, as Detent carries them: for
/// things that move under the keys and should feel it. Bezel's own roles are
/// for touch, where the finger carries the motion; with keys the spring has
/// to give it all, so these run a little past and come back. The calm
/// version damps them critically, as every spring there is.
pub mod spatial {
    use detent::Spring;

    /// `m3-expressive-fast-spatial` (stiffness 800, damping ratio 0.6): small
    /// moves with a visible bounce. About 9% over, peaking at 0.14 s,
    /// settled within 0.4 s.
    pub fn fast() -> Spring {
        Spring::stiffness_ratio(800.0, 0.6)
    }

    /// `m3-expressive-default-spatial` (380, 0.8): the everyday move. 1.5%
    /// over (5 px on a page turn), settled within 0.46 s.
    pub fn default() -> Spring {
        Spring::stiffness_ratio(380.0, 0.8)
    }

    /// `m3-expressive-slow-spatial` (200, 0.8): large moves, a whole page
    /// arriving. 1.5% over, settled within 0.64 s, as Bezel's page turn.
    pub fn slow() -> Spring {
        Spring::stiffness_ratio(200.0, 0.8)
    }
}

/// Where the ring goes: the fast spatial spring, so it overshoots a row by
/// about 2 px and draws back.
pub fn ring_spring() -> Spring {
    spatial::fast()
}

/// The ring's size and corners, changing between things of different
/// sizes: a shape growing with a little give.
pub fn ring_shape_spring() -> Spring {
    spatial::default()
}

/// Moving, the ring stretches along its motion and its tail trails, as
/// Bezel's dock droplet does: 0.04 × speed, at most 30 of Bezel's 720-wide
/// panel (13 px here), and never more than a third of its own length.
const RING_STRETCH: f32 = 0.04;
const RING_STRETCH_MAX: f32 = 30.0 * W / 720.0;

/// The ring at rest at `r`, stretched by the speed it is moving at (`vx`,
/// `vy`, px a second). Its leading edge is where the spring has it; the
/// stretch trails behind, and it thins a little across its motion (half the
/// stretch, at most 8% of its width). The droplet keeps its volume; a row's
/// ring is far wider than tall, so keeping its area would pinch it — this
/// thinning is my own tuning.
fn stretched(r: Rect, vx: f32, vy: f32) -> Rect {
    let along = |v: f32, size: f32| (v.abs() * RING_STRETCH).min(RING_STRETCH_MAX).min(size / 3.0);
    let (sx, sy) = (along(vx, r.w), along(vy, r.h));
    let (thin_w, thin_h) = ((sy * 0.5).min(r.w * 0.08), (sx * 0.5).min(r.h * 0.08));
    let x = if vx > 0.0 { r.x - sx } else { r.x } + thin_w * 0.5;
    let y = if vy > 0.0 { r.y - sy } else { r.y } + thin_h * 0.5;
    Rect::new(x, y, r.w + sx - thin_w, r.h + sy - thin_h)
}

impl Focus {
    fn new() -> Self {
        let at = |spring: Spring| Animated::new(0.0, spring).with_rest(0.05);
        Focus {
            x: at(ring_spring()),
            y: at(ring_spring()),
            w: at(ring_shape_spring()),
            h: at(ring_shape_spring()),
            radius: at(ring_shape_spring()),
            placed: false,
            along: 0.0,
            riding: false,
        }
    }

    fn springs(&mut self) -> [&mut Animated; 5] {
        [&mut self.x, &mut self.y, &mut self.w, &mut self.h, &mut self.radius]
    }

    /// The speed of the ring's middle, px a second.
    fn velocity(&self) -> (f32, f32) {
        (self.x.velocity() + 0.5 * self.w.velocity(), self.y.velocity() + 0.5 * self.h.velocity())
    }
}

impl Gfx {
    pub fn new(tokens: Tokens) -> Self {
        Gfx {
            canvas: Canvas::new(W as i32, H as i32),
            text: Text::new(),
            icons: Icons::new(),
            objects: crate::object3d::Objects::shipped(),
            missing_icons: Default::default(),
            glass: Glass::new(),
            tokens,
            accent: bezel::Accent::Orange,
            ty: Type::of(TextSize::Standard),
            text_size: TextSize::Standard,
            ox: 0.0,
            calm: false,
            home: crate::shell::HomeStyle::Grid,
            open_robot: true,
            match_report: true,
            glance: crate::shell::Settings::default().glance,
            pinned: None,
            arrangement: crate::shell::Arrangement::default(),
            clock: (true, false),
            island: crate::shell::IslandRest::Robot,
            greeting: true,
            developer: false,
            motion_scale: 1.0,
            show_keys: false,
            screen_off: 60,
            power: crate::standby::Power::Balanced,
            battery: crate::power::Choices::default(),
            lift_wake: true,
            face_down: true,
            lock: crate::lock::Choices::default(),
            look: crate::look::Look::default(),
            theme: crate::look::Theme::Catalyst,
            corner: 1.0,
            columns: 3,
            pager: None,
            title_room: 96.0,
            focus: Focus::new(),
            last_ring: Rect::default(),
            focus_static: false,
            motion_started: false,
            reveal: None,
            gpu: None,
            gpu_wanted: None,
            gpu_awaiting_device: false,
            gpu_opening: None,
            press: Animated::new(0.0, role::settle()).with_rest(0.01),
            scroll: Animated::new(0.0, role::hold()).with_rest(0.004),
            scroll_placed: false,
            ring_shift: 0.0,
        }
    }

    pub fn set_text_size(&mut self, size: TextSize) {
        self.text_size = size;
        self.ty = Type::of(size);
        // Catalyst's titles, as setup's: a step larger and a touch heavier.
        if self.theme == crate::look::Theme::Catalyst {
            self.ty.title = TextStyle { size: self.ty.title.size + 1.0, wght: 650.0, wdth: 100.0, ..self.ty.title };
        }
    }

    pub fn set_look(&mut self, look: crate::look::Look, columns: usize) {
        self.look = look;
        self.theme = look.theme;
        self.corner = look.corners.scale();
        self.columns = columns;
        self.text.set_letters(look.letters.round(), look.width.scale());
    }

    /// A corner radius as the corner setting makes it.
    pub fn radius(&self, base: f32) -> f32 {
        base * self.corner
    }

    /// The radius of a tile or a card.
    pub fn tile_radius(&self) -> f32 {
        self.radius(TILE_RADIUS)
    }

    /// The colour of a fault: the signal itself where it is already a red or
    /// an orange (Bezel's orange, Catalyst's crimson, and the own colours
    /// near them), otherwise a fixed red, so an error never reads in an
    /// accent's calm blue or green. #ff6b5e on dark surfaces (at least 4.8:1
    /// on every one), Material's #b3261e on light (at least 5:1).
    pub fn fault(&self) -> Color {
        let s = self.tokens.signal;
        let (r, g, b) = (s.0 as f32, s.1 as f32, s.2 as f32);
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let warm = max == r && max - min > 40.0 && {
            let hue = 60.0 * ((g - b) / (max - min)).rem_euclid(6.0);
            !(45.0..330.0).contains(&hue)
        };
        match (warm, self.tokens.tone) {
            (true, _) => s,
            (false, bezel::Tone::Dark) => Color::hex(0xff6b5e),
            (false, bezel::Tone::Light) => Color::hex(0xb3261e),
        }
    }

    /// Words that read as "fine" (a 200, the fastest lap): ice on dark, and
    /// ice's own ink on light, where ice is a fill too pale to read.
    pub fn good(&self) -> Color {
        match self.tokens.tone {
            bezel::Tone::Dark => self.tokens.ice,
            bezel::Tone::Light => self.tokens.on_ice,
        }
    }

    /// The focus ring's colour: in Catalyst, the ink (white on dark), as
    /// setup's; in Bezel, the signal.
    pub fn ring_color(&self) -> Color {
        match self.theme {
            crate::look::Theme::Catalyst => self.tokens.ink,
            crate::look::Theme::Bezel => self.tokens.signal,
        }
    }

    // ---- layout, measured from the type ----

    /// The glass island's height.
    pub fn island_h(&self) -> f32 {
        self.ty.caption.size + 8.0
    }
    /// Where a screen's content starts, below the island.
    pub fn top(&self) -> f32 {
        self.island_h() + 9.0
    }
    /// The height of a one-line list row.
    pub fn row_h(&self) -> f32 {
        self.ty.body.size + 10.0
    }
    /// The space a hint line takes at the bottom of a page.
    pub fn hint_h(&self) -> f32 {
        self.ty.caption.size + 4.0
    }

    // ---- drawing ----

    pub fn surface(&mut self, r: Rect, radius: f32, color: Color) {
        self.canvas.rrect(r.offset(self.ox, 0.0), radius, color, 1.0);
    }

    pub fn text(&mut self, x: f32, baseline: f32, s: &str, style: TextStyle, color: Color) -> f32 {
        self.text.draw(&mut self.canvas, x + self.ox, baseline, s, &style, color, 1.0)
    }

    /// Text placed by its left, centre or right, centred vertically on `cy`.
    pub fn text_at(&mut self, anchor: f32, cy: f32, align: Align, s: &str, style: TextStyle, color: Color) -> f32 {
        self.text.draw_aligned(&mut self.canvas, anchor + self.ox, baseline_for(&style, cy), align, s, &style, color, 1.0)
    }

    /// Left-aligned text centred on `cy`, cut with an ellipsis to fit `max_width`.
    pub fn text_fit(&mut self, x: f32, cy: f32, s: &str, style: TextStyle, color: Color, max_width: f32) -> f32 {
        let fitted = self.text.fit(s, &style, max_width.max(0.0));
        self.text_at(x, cy, Align::Left, &fitted, style, color)
    }

    pub fn measure(&mut self, s: &str, style: TextStyle) -> f32 {
        self.text.measure(s, &style)
    }

    pub fn icon(&mut self, name: &str, filled: bool, cx: f32, cy: f32, size: f32, color: Color) {
        self.icon_alpha(name, filled, cx, cy, size, color, 1.0);
    }

    /// An icon at an opacity.
    #[allow(clippy::too_many_arguments)]
    pub fn icon_alpha(&mut self, name: &str, filled: bool, cx: f32, cy: f32, size: f32, color: Color, alpha: f32) {
        let drawn = self.icons.draw(&mut self.canvas, name, filled, cx + self.ox, cy, size, color, alpha);
        if !drawn && !name.is_empty() && !self.missing_icons.contains(name) {
            eprintln!("catalyst-shell: no icon called {name:?}");
            self.missing_icons.insert(name.to_owned());
        }
    }

    /// The icon names asked for that are not built in.
    pub fn missing_icons(&self) -> &std::collections::BTreeSet<String> {
        &self.missing_icons
    }

    /// A 3D object (`object3d`), centred on (`cx`, `cy`), turned to `view`,
    /// `scale` times the size it was rendered at. False if there is none of
    /// that name, so the caller can draw its icon instead.
    #[allow(clippy::too_many_arguments)]
    pub fn object(&mut self, name: &str, size: &str, cx: f32, cy: f32, view: crate::object3d::View, scale: f32, alpha: f32) -> bool {
        let fault = self.fault();
        self.objects.draw(&mut self.canvas, &self.tokens, fault, name, size, cx + self.ox, cy, view, scale, alpha)
    }

    /// How fast the focus ring is moving just now, px a second (0 when calm
    /// or drawn as a still picture): what turns the focused object with it.
    pub fn ring_velocity(&self) -> (f32, f32) {
        if self.calm || self.focus_static || !self.focus.placed {
            return (0.0, 0.0);
        }
        self.focus.velocity()
    }

    /// Drawing set something moving: frames should follow at once.
    pub fn started_moving(&mut self) {
        self.motion_started = true;
    }

    pub fn dot(&mut self, cx: f32, cy: f32, radius: f32, color: Color) {
        self.canvas.circle(cx + self.ox, cy, radius, color, 1.0);
    }

    /// The focus ring: signal, just outside the focused surface. It glides
    /// from the last thing focused to this one with a little overshoot,
    /// stretching along its way and drawing itself together as it lands; its
    /// size and corners follow too. (Bezel itself lands changes made from
    /// keys at once; the calm version keeps to that.)
    pub fn focus_ring(&mut self, r: Rect, radius: f32) {
        self.ring_at(r, radius, None);
    }

    /// The focus ring on a strip that slides sideways as a whole (Home's
    /// pages of tiles): `along` is how far the strip is moved just now. The
    /// ring glides along the strip and rides with it as it turns, rather than
    /// chasing a tile that moves every frame.
    pub fn focus_ring_on(&mut self, r: Rect, radius: f32, along: f32) {
        self.ring_at(r, radius, Some(along));
    }

    fn ring_at(&mut self, r: Rect, radius: f32, strip: Option<f32>) {
        let signal = self.ring_color();
        let r = r.offset(0.0, self.ring_shift);
        let (at, radius) = if self.focus_static {
            (r, radius)
        } else {
            let along = strip.unwrap_or(0.0);
            let rest = r.offset(-along, 0.0);
            let f = &mut self.focus;
            // Onto a strip from the screen, or off it: the ring's place is
            // measured from somewhere else now, and where it is drawn stays.
            if f.placed && f.riding != strip.is_some() {
                f.x.shift(f.along - along);
            }
            f.riding = strip.is_some();
            f.along = along;
            let want = [rest.x, rest.y, rest.w, rest.h, radius];
            if !f.placed || self.calm {
                for (spring, to) in f.springs().into_iter().zip(want) {
                    spring.jump(to);
                }
                f.placed = true;
            } else if f.springs().iter().zip(want).any(|(spring, to)| spring.target() != to) {
                for (spring, to) in f.springs().into_iter().zip(want) {
                    spring.set(to);
                }
                self.motion_started = true;
            }
            let (vx, vy) = f.velocity();
            let at = Rect::new(f.x.value() + along, f.y.value(), f.w.value(), f.h.value());
            (if self.calm { at } else { stretched(at, vx, vy) }, f.radius.value())
        };
        let ring = at.inset(-2.5);
        let radius = (radius + 2.5).min(ring.w * 0.5).min(ring.h * 0.5).max(0.0);
        self.last_ring = ring;
        self.canvas.stroke_rrect(ring.offset(self.ox, 0.0), radius, 1.5, signal, 1.0);
    }

    /// Forget where the ring was: the next screen's focus is placed at once.
    pub fn focus_reset(&mut self) {
        self.focus.placed = false;
        self.scroll_placed = false;
    }

    /// Where the list on screen is scrolled to just now, in rows, given that
    /// its first row should come to be `first`. `bump` is a push against an
    /// end of the list (−1 the top, +1 the bottom): the rows give a little
    /// and spring back, which says "no further" without a word.
    fn list_scroll(&mut self, first: f32, bump: f32) -> f32 {
        if self.focus_static {
            return first;
        }
        if !self.scroll_placed || self.calm {
            self.scroll.jump(first);
            self.scroll_placed = true;
        } else if self.scroll.target() != first {
            self.scroll.set(first);
            self.motion_started = true;
        }
        if bump != 0.0 && !self.calm {
            self.scroll.nudge(bump * 22.0);
            self.motion_started = true;
        }
        self.scroll.value()
    }

    /// Advance what moves on its own. True while something still is.
    pub fn tick(&mut self, dt: f32) -> bool {
        // Each spring must be advanced, so no short-circuit.
        let mut moving = self.scroll.tick(dt) | self.press.tick(dt);
        for spring in self.focus.springs() {
            moving |= spring.tick(dt);
        }
        if let Some(t) = &mut self.reveal {
            *t += dt;
            if *t > REVEAL_OVER {
                self.reveal = None;
            }
            moving = true;
        }
        moving
    }

    /// A page has begun to arrive: its rows rise into place.
    pub fn reveal_start(&mut self) {
        self.reveal = (!self.calm).then_some(0.0);
    }

    /// The arriving page turned back: whatever is left of its rise is dropped.
    pub fn reveal_stop(&mut self) {
        self.reveal = None;
    }

    /// Whether the page in front is still rising into place.
    pub fn revealing(&self) -> bool {
        self.reveal.is_some()
    }

    /// Where a block of the arriving page is, by its place in the order its
    /// blocks arrive in (the title first): how far below its place it still
    /// is, or `None` if its turn has not come (it is not drawn yet).
    pub fn rise(&self, order: f32) -> Option<f32> {
        let t = match self.reveal {
            Some(t) if !self.calm => t,
            _ => return Some(0.0),
        };
        let since = t - order * STAGGER;
        (since >= 0.0).then(|| RISE * role::release().solve(1.0, 0.0, since).0)
    }

    /// Enter landed on whatever has the focus.
    pub fn pressed(&mut self) {
        if !self.calm {
            self.press.jump(1.0);
            self.press.set(0.0);
        }
    }

    /// How pressed the focused thing is: 1 as the key lands, 0 at rest.
    pub fn press(&self) -> f32 {
        self.press.value().clamp(0.0, 1.0)
    }

    /// Whether drawing just gave something a new place to move to.
    pub fn take_motion_started(&mut self) -> bool {
        std::mem::take(&mut self.motion_started)
    }

    /// Left-aligned text centred on `cy`, at an opacity.
    pub fn text_alpha(&mut self, x: f32, cy: f32, s: &str, style: TextStyle, color: Color, alpha: f32) {
        self.text.draw(&mut self.canvas, x + self.ox, baseline_for(&style, cy), s, &style, color, alpha);
    }

    /// A meter: the fill takes the track's outer corners through a clip.
    pub fn meter(&mut self, r: Rect, fraction: f32, fill: Color) {
        let track = r.offset(self.ox, 0.0);
        let radius = r.h * 0.5;
        self.canvas.rrect(track, radius, self.tokens.meter, 1.0);
        let fraction = fraction.clamp(0.0, 1.0);
        if fraction > 0.0 {
            let w = (track.w * fraction).max(r.h * 0.6);
            self.canvas.rrect_clipped(Rect::new(track.x, track.y, w, track.h), 2.0, track, radius, fill, 1.0);
        }
    }

    /// A tile holding one glanceable value: a quiet label above, the value in
    /// the display voice, and its unit.
    pub fn stat(&mut self, r: Rect, label: &str, value: Option<String>, unit: &str, size: f32) {
        let t = self.tokens;
        let ty = self.ty;
        self.surface(r, self.tile_radius(), t.surface1);
        self.text_at(r.x + 9.0, r.y + 5.0 + ty.caption.size * 0.5, Align::Left, label, ty.caption, t.dim);
        let display = ty.display(size);
        let baseline = r.bottom() - 8.0;
        match value {
            Some(v) => {
                let w = self.text(r.x + 8.0, baseline, &v, display, t.ink);
                self.text(r.x + 10.0 + w, baseline, unit, ty.caption, t.dim);
            }
            // Absent, never 0.
            None => {
                self.text(r.x + 8.0, baseline, "–", display, t.faint);
            }
        }
    }

    /// One row of a list: a name on the left, a mono value on the right.
    pub fn row(&mut self, r: Rect, name: &str, value: &str, value_color: Color, focused: bool) {
        let t = self.tokens;
        let ty = self.ty;
        // Catalyst's rows are capsules, as setup's; Bezel's are squarer tonal
        // panels, named lowercase in the mono voice with their value in Sans.
        let bezel = self.theme == crate::look::Theme::Bezel;
        let radius = if bezel { self.radius(7.0).min(r.h * 0.5) } else { r.h * 0.5 };
        // The row the focus ring is over lifts: a touch larger, lighter, with a
        // soft shadow. Following the ring as it glides, the row it leaves
        // settles as the next one rises, with no springs of their own.
        let lift = self.lift_under_ring(r, focused);
        let press = if focused { self.press() } else { 0.0 };
        let lifted = r.inset(-1.0 * lift + 1.5 * press);
        let corner = (radius + lift - 1.5 * press).max(0.0);
        self.shadow(lifted, corner, lift);
        let fill = t.surface1.mix(t.surface2, lift).mix(t.ink, 0.2 * press);
        self.surface(lifted, corner, fill);
        if bezel {
            let value_w = self.measure(value, ty.body);
            self.text_fit(r.x + 10.0, r.cy(), &name.to_lowercase(), ty.label, t.ink, r.w - 30.0 - value_w);
            self.text_at(r.right() - 10.0, r.cy(), Align::Right, value, ty.body, value_color);
        } else {
            let value_w = self.measure(value, ty.label);
            self.text_fit(r.x + 10.0, r.cy(), name, ty.body, t.ink, r.w - 30.0 - value_w);
            self.text_at(r.right() - 10.0, r.cy(), Align::Right, value, ty.label, value_color);
        }
        if focused {
            self.focus_ring(r, radius);
        }
    }

    /// How lifted a row is: 1 under the focus ring, falling to 0 a row away
    /// from it. Drawn as a still picture, or in the calm version, the
    /// focused row alone, wholly.
    fn lift_under_ring(&self, r: Rect, focused: bool) -> f32 {
        let f = &self.focus;
        if self.focus_static || self.calm || !f.placed {
            return if focused { 1.0 } else { 0.0 };
        }
        let (cx, cy) = (f.x.value() + f.w.value() * 0.5, f.y.value() + f.h.value() * 0.5);
        if (cx - r.cx()).abs() > r.w * 0.5 {
            return 0.0;
        }
        (1.0 - (cy - r.cy()).abs() / r.h).clamp(0.0, 1.0)
    }

    /// A soft shadow under something lifted: three layers, each a little
    /// wider and lower and half as dark, lit from above.
    pub fn shadow(&mut self, r: Rect, radius: f32, strength: f32) {
        if strength <= 0.01 {
            return;
        }
        let base = if self.tokens.tone == bezel::Tone::Dark { 0.34 } else { 0.10 };
        for i in 0..3 {
            let k = i as f32;
            self.canvas.rrect(r.offset(self.ox, 1.5 + k).inset(-k), radius + k, Color::BLACK, base * strength * 0.5f32.powi(i));
        }
    }

    /// The hint line at the bottom of a page: lowercase mono, quiet.
    /// The line along the bottom: what the keys do, or how things stand.
    /// Always whole on the panel: a long one is cut short with an ellipsis.
    pub fn hint(&mut self, s: &str) {
        let (style, color) = (self.ty.caption, self.tokens.dim);
        let fitted = self.text.fit(s, &style, W - 2.0 * PAD);
        self.text_at(W * 0.5, H - 2.0 - style.size * 0.5, Align::Center, &fitted, style, color);
    }

    /// The first of `choices` (longest first) that fits whole, so a larger
    /// text size leaves keys out rather than cutting one in half.
    pub fn hint_any(&mut self, choices: &[&str]) {
        let style = self.ty.caption;
        let fits = choices.iter().copied().find(|s| self.measure(s, style) <= W - 2.0 * PAD);
        if let Some(s) = fits.or(choices.last().copied()) {
            self.hint(s);
        }
    }

    /// What a page says when it has nothing to show: its object in three
    /// dimensions where there is one (`object3d::for_empty`), turning into
    /// place as the page arrives, otherwise its icon; and the line under it,
    /// cut short rather than run off the panel.
    pub fn empty(&mut self, icon: &str, line: &str) {
        let t = self.tokens;
        let cy = (self.top() + H) * 0.5;
        let style = self.ty.caption;
        let line = self.text.fit(line, &style, W - 2.0 * PAD);
        if let Some((object, size)) = crate::object3d::for_empty(icon, &self.objects) {
            // Swinging round from a quarter turn away as the page rises in.
            let turn = match self.reveal {
                Some(since) if !self.calm => 40.0 * role::release().solve(1.0, 0.0, since).0,
                _ => 0.0,
            };
            if self.object(object, size, W * 0.5, cy - 13.0, crate::object3d::View::yaw(turn), 1.0, 1.0) {
                self.text_at(W * 0.5, cy + 24.0, Align::Center, &line, style, t.dim);
                return;
            }
        }
        self.icon(icon, false, W * 0.5, cy - 12.0, 24.0, t.faint);
        self.text_at(W * 0.5, cy + 12.0, Align::Center, &line, style, t.dim);
    }

    /// Run the glass pass over everything drawn so far: on the GPU when it
    /// is drawing, otherwise (or for glass it leaves) on the processor.
    pub fn glass(&mut self, shapes: &[GlassShape]) {
        if let Some(gpu) = &mut self.gpu {
            match gpu.glass(&mut self.canvas, shapes, &self.tokens, bezel::LIGHT_REST, self.calm) {
                Ok(true) => return,
                Ok(false) => {}
                Err(why) => self.gpu_failed(why),
            }
        }
        self.glass.render(&mut self.canvas, shapes, &self.tokens, bezel::LIGHT_REST, self.calm);
    }

    /// Whether the GPU should draw (Settings › Display › Graphics). Opening
    /// it happens here, at once, only when the wish changes.
    pub fn want_gpu(&mut self, on: bool) {
        let forced = std::env::var("CATALYST_GPU").ok();
        let on = on && forced.as_deref() != Some("off");
        if self.gpu_wanted == Some(on) {
            return;
        }
        self.gpu_wanted = Some(on);
        if !on {
            self.gpu = None;
            self.gpu_opening = None;
            self.gpu_awaiting_device = false;
            crate::gpu::set_choice("processor · your choice", "processor · chosen in Settings › Display › Graphics");
            return;
        }
        self.try_gpu();
    }

    pub fn gpu_wanted(&self) -> bool {
        self.gpu_wanted == Some(true)
    }

    /// Open the GPU on a thread of its own: loading Mesa, compiling the
    /// shaders and the test frame take a moment the start animation should
    /// not wait for. `tick_gpu` takes the result up when it comes.
    fn try_gpu(&mut self) {
        let software = std::env::var("CATALYST_GPU").is_ok_and(|v| v == "software");
        let (send, receive) = std::sync::mpsc::channel();
        let opening = std::thread::Builder::new().name("gpu".into()).spawn(move || {
            let opened = crate::gpu::Gpu::open(software).inspect(|gpu| gpu.release());
            send.send(opened).ok();
        });
        if opening.is_ok() {
            self.gpu_opening = Some(receive);
            self.gpu_awaiting_device = false;
            crate::gpu::set_choice("processor · trying gpu", "processor, while the gpu is tried");
        }
    }

    /// Wait for the GPU to finish opening (for `--about` and `--bench`).
    pub fn wait_gpu(&mut self) {
        if let Some(Ok(opened)) = self.gpu_opening.take().map(|r| r.recv()) {
            self.gpu_opened(opened);
        }
    }

    fn gpu_opened(&mut self, opened: Result<crate::gpu::Gpu, String>) {
        // `software`: Mesa's software renderer allowed and every job given to
        // it, to test the path; `force`: every job to the GPU, quicker or not.
        let forced = std::env::var("CATALYST_GPU").ok();
        let software = forced.as_deref() == Some("software");
        match opened.and_then(|gpu| gpu.adopt().map(|()| gpu)) {
            Ok(_) if !self.gpu_wanted() => {}
            Ok(mut gpu) => {
                if software || forced.as_deref() == Some("force") {
                    gpu.use_everything();
                }
                if !gpu.worth_it() {
                    eprintln!("catalyst-shell: drawing with the processor, quicker than the gpu here ({})", gpu.jobs());
                    crate::gpu::set_choice("processor · faster here", format!("processor · faster than the {} here, reading back included: {}", gpu.renderer(), gpu.jobs()));
                    return;
                }
                eprintln!("catalyst-shell: drawing with the gpu ({}; {})", crate::gpu::note(), gpu.jobs());
                crate::gpu::set_choice("gpu", format!("{} · {}", gpu.renderer(), gpu.jobs()));
                self.gpu = Some(gpu);
            }
            Err(why) => {
                eprintln!("catalyst-shell: drawing with the processor: {why}");
                crate::gpu::set_choice(crate::gpu::why_not(&why), format!("processor · {why}"));
                // The kernel's GPU driver may still be loading: try once more when its device appears.
                self.gpu_awaiting_device = why == "no gpu device";
            }
        }
    }

    /// The GPU went wrong mid-frame: the processor draws from now on.
    pub fn gpu_failed(&mut self, why: String) {
        eprintln!("catalyst-shell: the gpu failed, drawing with the processor: {why}");
        crate::gpu::set_choice("processor · gpu failed", format!("processor · the gpu failed while drawing: {why}"));
        self.gpu = None;
        self.gpu_awaiting_device = false;
    }

    /// Every frame: has the GPU finished opening?
    pub fn tick_gpu(&mut self) {
        let Some(opening) = &self.gpu_opening else { return };
        match opening.try_recv() {
            Ok(opened) => {
                self.gpu_opening = None;
                self.gpu_opened(opened);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.gpu_opening = None,
        }
    }

    /// Now and then, while the GPU is wanted and its device has not
    /// appeared yet: has it now?
    pub fn look_for_gpu(&mut self) {
        if self.gpu_awaiting_device && self.gpu_wanted() && self.gpu.is_none() && self.gpu_opening.is_none() && crate::gpu::device_present() {
            self.try_gpu();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where row `index` is drawn, if it is.
    fn row_y(rows: &[(usize, f32)], index: usize) -> Option<f32> {
        rows.iter().find(|(i, _)| *i == index).map(|(_, y)| *y)
    }

    #[test]
    fn a_fault_reads_red_whatever_the_accent() {
        let mut g = Gfx::new(Tokens::dark());
        let accents = bezel::Accent::ALL.into_iter().chain((0..bezel::Accent::HUES).map(bezel::Accent::Hue));
        for accent in accents {
            for tone in [bezel::Tone::Dark, bezel::Tone::Light] {
                g.tokens = Tokens::of(accent, tone);
                let c = g.fault();
                assert!(c.0 as i32 > c.1 as i32 + 40 && c.0 as i32 > c.2 as i32 + 40, "{accent:?} {tone:?}: {c:?}");
            }
        }
        // Bezel's own orange and Catalyst's crimson keep their signal for it.
        g.tokens = Tokens::of(bezel::Accent::Rose, bezel::Tone::Dark);
        assert_eq!(g.fault(), g.tokens.signal);
        g.tokens = Tokens::of(bezel::Accent::Blue, bezel::Tone::Dark);
        assert_ne!(g.fault(), g.tokens.signal);
    }

    const FRAME: f32 = 1.0 / 40.0;

    /// Row `i` of a list: 18 px tall, 20 px apart.
    fn row(i: usize) -> Rect {
        Rect::new(10.0, 30.0 + 20.0 * i as f32, 280.0, 18.0)
    }

    /// Frames at the handheld's beat while the ring moves, drawing it on
    /// `at` each time; the ring as drawn in each.
    fn glide(g: &mut Gfx, at: Rect) -> Vec<Rect> {
        let mut seen = Vec::new();
        while g.tick(FRAME) {
            g.focus_ring(at, 6.0);
            seen.push(g.last_ring);
            assert!(seen.len() < 80, "the ring never came to rest");
        }
        g.focus_ring(at, 6.0);
        seen.push(g.last_ring);
        seen
    }

    #[test]
    fn the_ring_runs_a_little_past_stretches_on_its_way_and_rests_exactly() {
        let mut g = Gfx::new(Tokens::dark());
        g.focus_ring(row(0), 6.0);
        assert_eq!(g.last_ring, row(0).inset(-2.5), "its first place is taken at once");
        g.focus_ring(row(1), 6.0);
        assert!(g.take_motion_started());
        let seen = glide(&mut g, row(1));
        let rest = row(1).inset(-2.5);
        // Its leading edge runs past where it rests, a couple of pixels, and comes back.
        let furthest = seen.iter().map(|r| r.bottom()).fold(f32::MIN, f32::max);
        assert!(furthest > rest.bottom() + 1.0 && furthest < rest.bottom() + 4.0, "over by {}", furthest - rest.bottom());
        // On its way it is drawn out along its motion, its tail behind it.
        let tallest = seen.iter().map(|r| r.h).fold(0.0, f32::max);
        assert!(tallest > rest.h + 3.0 && tallest <= rest.h + rest.h / 3.0 + 1.0, "{} tall at most, {} at rest", tallest, rest.h);
        // And it settles exactly, within half a second at the handheld's beat
        // (to a twentieth of a pixel: the shell then stops drawing).
        assert_eq!(*seen.last().unwrap(), rest);
        assert!(seen.len() as f32 * FRAME <= 0.5, "{} frames", seen.len());
        assert!(!g.tick(FRAME), "and then nothing moves");
    }

    #[test]
    fn the_ring_keeps_its_speed_when_the_focus_moves_on_before_it_lands() {
        let mut g = Gfx::new(Tokens::dark());
        g.focus_ring(row(0), 6.0);
        g.focus_ring(row(1), 6.0);
        for _ in 0..3 {
            g.tick(FRAME);
            g.focus_ring(row(1), 6.0);
        }
        let (before, at) = (g.focus.velocity().1, g.last_ring.y);
        assert!(before > 100.0, "well on its way: {before} px/s");
        // Down again (a key held): it heads on without starting over.
        g.focus_ring(row(2), 6.0);
        assert!((g.focus.velocity().1 - before).abs() < 1.0, "same speed: {} then {}", before, g.focus.velocity().1);
        g.tick(FRAME);
        g.focus_ring(row(2), 6.0);
        assert!(g.last_ring.y > at, "still going the same way");
        let seen = glide(&mut g, row(2));
        assert_eq!(*seen.last().unwrap(), row(2).inset(-2.5));
        // Back up mid-glide: it turns round from where it is.
        g.focus_ring(row(4), 6.0);
        g.tick(FRAME);
        g.focus_ring(row(4), 6.0);
        let turning = g.last_ring;
        g.focus_ring(row(3), 6.0);
        g.tick(FRAME);
        g.focus_ring(row(3), 6.0);
        assert!((g.last_ring.y - turning.y).abs() < 20.0, "no jump: {} to {}", turning.y, g.last_ring.y);
    }

    #[test]
    fn the_ring_rides_a_turning_strip_and_keeps_its_place_coming_on_and_off_it() {
        let mut g = Gfx::new(Tokens::dark());
        let tile = Rect::new(112.0, 30.0, 96.0, 60.0);
        g.focus_ring_on(tile, 10.0, 0.0);
        // The strip turns a page while the focus stays on the tile: the ring
        // is on the tile every frame, never behind it.
        for step in 1..=10 {
            let along = -32.0 * step as f32;
            g.tick(FRAME);
            g.focus_ring_on(tile.offset(along, 0.0), 10.0, along);
            assert_eq!(g.last_ring, tile.offset(along, 0.0).inset(-2.5), "step {step}");
        }
        // Off the strip onto the screen (the glance page's cards): drawn where it was.
        let before = g.last_ring;
        g.focus_ring(Rect::new(8.0, 30.0, 150.0, 120.0), 10.0);
        assert!((g.last_ring.x - before.x).abs() < 1.0, "{} then {}", before.x, g.last_ring.x);
    }

    #[test]
    fn the_calm_ring_lands_at_once_without_stretching() {
        let mut g = Gfx::new(Tokens::dark());
        g.calm = true;
        g.focus_ring(row(0), 6.0);
        g.focus_ring(row(3), 6.0);
        assert_eq!(g.last_ring, row(3).inset(-2.5));
        assert!(!g.tick(FRAME));
    }

    #[test]
    fn a_list_glides_to_where_it_is_scrolled_and_gives_at_its_end() {
        let mut g = Gfx::new(Tokens::dark());
        let mut list = ListState::default();
        // Twenty rows, 20 px apart, four to a page starting at y = 30.
        let layout = |g: &mut Gfx, list: &mut ListState| {
            let rows = list.layout(g, 20, 30.0, 18.0, 2.0, 110.0);
            list.scrollbar(g, 20, 30.0, 110.0);
            rows
        };
        let first = layout(&mut g, &mut list);
        assert_eq!(row_y(&first, 0), Some(30.0));
        // Down past the bottom of the page: the list has to move up one row.
        for _ in 0..4 {
            list.down(20);
        }
        let just_after = layout(&mut g, &mut list);
        assert_eq!(row_y(&just_after, 0), Some(30.0), "nothing has jumped yet");
        g.tick(1.0 / 60.0);
        let moving = layout(&mut g, &mut list);
        let y = row_y(&moving, 1).unwrap();
        assert!(y < 50.0 && y > 30.0, "part-way through its glide: {y}");
        // A row coming in at the bottom is drawn while it is only partly in view.
        assert!(row_y(&moving, 4).is_some_and(|y| y > 90.0));
        for _ in 0..60 {
            g.tick(1.0 / 60.0);
        }
        let settled = layout(&mut g, &mut list);
        assert_eq!(row_y(&settled, 1), Some(30.0));
        assert_eq!(row_y(&settled, 0), None, "the first row has gone off the top");

        // At the last row, Down again: the rows give a little and come back.
        for _ in 0..30 {
            list.down(20);
        }
        layout(&mut g, &mut list);
        for _ in 0..60 {
            g.tick(1.0 / 60.0);
        }
        let rest = row_y(&layout(&mut g, &mut list), 19).unwrap();
        list.down(20);
        layout(&mut g, &mut list);
        g.tick(1.0 / 30.0);
        let pushed = row_y(&layout(&mut g, &mut list), 19).unwrap();
        assert!(pushed < rest - 0.5 && pushed > rest - 12.0, "a small give: {rest} to {pushed}");
        for _ in 0..60 {
            g.tick(1.0 / 60.0);
        }
        assert_eq!(row_y(&layout(&mut g, &mut list), 19), Some(rest));
        // The calm version moves at once and does not give.
        g.calm = true;
        list.up();
        list.page(20, -1);
        let calm = layout(&mut g, &mut list);
        assert!(calm.iter().all(|(_, y)| (y - 30.0) % 20.0 == 0.0));
    }
}

/// A focus index over a scrolling list of fixed-height rows.
#[derive(Default)]
pub struct ListState {
    pub focus: usize,
    first: usize,
    visible: usize,
    /// A key pressed against an end of the list: −1 at the top, +1 at the bottom.
    bump: f32,
    /// Where the rows were last drawn, in rows scrolled: the glide's position.
    shown: f32,
    /// Whether this list has been drawn yet. A new one (a folder entered, a
    /// page reset) starts where it is, rather than gliding from wherever the
    /// list before it had scrolled to.
    placed: bool,
}

impl ListState {
    pub fn up(&mut self) {
        if self.focus == 0 {
            self.bump = -1.0;
        }
        self.focus = self.focus.saturating_sub(1);
    }
    pub fn down(&mut self, len: usize) {
        if self.focus + 1 < len {
            self.focus += 1;
        } else {
            self.bump = 1.0;
        }
    }
    /// Move by a page: as many rows as were last on screen.
    pub fn page(&mut self, len: usize, direction: isize) {
        let step = self.visible.max(1) as isize * direction;
        self.focus = (self.focus as isize + step).clamp(0, len.saturating_sub(1) as isize) as usize;
    }
    /// The rows to draw: (index, y). Keeps the focus on screen, and glides
    /// there: while the list is moving, a row may be part-way in at the top
    /// or bottom, so drawing is clipped to the list's area until
    /// [`ListState::scrollbar`] is called, which every list does last.
    pub fn layout(&mut self, g: &mut Gfx, len: usize, top: f32, row_h: f32, gap: f32, bottom: f32) -> Vec<(usize, f32)> {
        self.focus = self.focus.min(len.saturating_sub(1));
        self.visible = (((bottom - top) + gap) / (row_h + gap)).floor().max(1.0) as usize;
        if self.focus < self.first {
            self.first = self.focus;
        } else if self.focus >= self.first + self.visible {
            self.first = self.focus + 1 - self.visible;
        }
        self.first = self.first.min(len.saturating_sub(self.visible));
        let pitch = row_h + gap;
        if !self.placed && !g.focus_static {
            g.scroll_placed = false;
            self.placed = true;
        }
        let scroll = g.list_scroll(self.first as f32, std::mem::take(&mut self.bump));
        self.shown = scroll;
        g.ring_shift = (scroll - self.first as f32) * pitch;
        // A little beyond the rows themselves, so the focus ring is whole.
        g.canvas.set_clip(Rect::new(0.0, top - 4.0, W, self.visible as f32 * pitch - gap + 8.0));
        let from = scroll.max(0.0).floor() as usize;
        let to = (scroll.max(0.0).ceil() as usize + self.visible).min(len);
        // An arriving page's rows rise into place one after another.
        (from..to).map(|i| (i, top + (i as f32 - scroll) * pitch)).collect()
    }
    /// A thin position marker when the list is longer than the page. Ends
    /// the clipping `layout` began.
    pub fn scrollbar(&self, g: &mut Gfx, len: usize, top: f32, bottom: f32) {
        g.canvas.clear_clip();
        g.ring_shift = 0.0;
        if len <= self.visible {
            return;
        }
        let track = bottom - top;
        let thumb = (track * self.visible as f32 / len as f32).max(10.0);
        let y = top + (track - thumb) * (self.shown / (len - self.visible) as f32).clamp(0.0, 1.0);
        let faint = g.tokens.faint;
        g.surface(Rect::new(W - 5.0, y, 2.5, thumb), 1.25, faint);
    }
}
