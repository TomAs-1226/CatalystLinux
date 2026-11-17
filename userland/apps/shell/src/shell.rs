//! The shell: a stack of screens, the glass island that carries link status
//! and messages, and the page turn between screens.

use bezel::{Accent, Align, GlassShape, Rect, Tokens, Tone};
use detent::{role, Animated};

use crate::gfx::{Gfx, TextSize, H, PAD, W};
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
    Push(Box<dyn Screen>),
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
}

const MESSAGE_SECONDS: f32 = 2.4;

pub struct Shell {
    pub gfx: Gfx,
    pub robot: Robot,
    pub settings: Settings,
    stack: Vec<Box<dyn Screen>>,
    /// The screen sliding away during a page turn.
    leaving: Option<Box<dyn Screen>>,
    /// 1 → 0 across a page turn; `forward` says which way.
    turn: Animated,
    forward: bool,
    island_w: Animated,
    message: Option<(String, f32)>,
    dirty: bool,
    screen_moving: bool,
    /// The ground and its glows, drawn once per palette and copied each frame.
    background: Vec<u32>,
    background_for: Option<(u32, u32, u32)>,
    /// A page turn slides two finished pictures instead of redrawing two
    /// screens every frame: the one underneath and the one on top, each with
    /// the page dots it asked for.
    turn_pictures: Option<[(Vec<u32>, Option<(usize, usize)>); 2]>,
}

impl Shell {
    pub fn new(robot: Robot) -> Self {
        Shell {
            gfx: Gfx::new(Tokens::dark()),
            robot,
            settings: Settings { light: false, calm: false, text: TextSize::Standard, accent: Accent::Orange },
            stack: vec![Box::new(Home::new())],
            leaving: None,
            turn: Animated::new(0.0, role::page()).with_rest(0.002),
            forward: true,
            island_w: Animated::new(60.0, role::release()).with_rest(0.2),
            message: None,
            dirty: true,
            screen_moving: false,
            background: Vec::new(),
            background_for: None,
            turn_pictures: None,
        }
    }

    pub fn say(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), MESSAGE_SECONDS));
        self.dirty = true;
    }

    pub fn key(&mut self, key: Key) {
        self.dirty = true;
        let top = self.stack.last_mut().expect("the home screen is always there");
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
            Nav::Push(screen) => {
                self.stack.push(screen);
                self.leaving = None;
                self.start_turn(true);
            }
            Nav::Pop if self.stack.len() > 1 => {
                self.leaving = self.stack.pop();
                self.start_turn(false);
            }
            Nav::Pop => {}
        }
        self.gfx.accent = self.settings.accent;
        self.gfx.tokens = Tokens::of(self.settings.accent, if self.settings.light { Tone::Light } else { Tone::Dark });
        self.gfx.calm = self.settings.calm;
        self.gfx.set_text_size(self.settings.text);
    }

    fn start_turn(&mut self, forward: bool) {
        self.forward = forward;
        self.turn_pictures = None;
        if self.settings.calm {
            self.turn.jump(0.0);
            self.leaving = None;
        } else {
            self.turn.jump(1.0);
            self.turn.set(0.0);
        }
    }

    /// Advance time. Returns whether a frame should be drawn.
    pub fn tick(&mut self, dt: f32) -> bool {
        let mut moving = self.turn.tick(dt);
        if !moving {
            self.leaving = None;
        }
        moving |= self.island_w.tick(dt);
        self.screen_moving = self.stack.last_mut().is_some_and(|s| s.tick(dt));
        moving |= self.screen_moving;
        if let Some((_, left)) = &mut self.message {
            *left -= dt;
            if *left <= 0.0 {
                self.message = None;
                self.dirty = true;
            }
        }
        let changed = self.robot.refresh(dt);
        let draw = moving || changed || self.dirty;
        self.dirty = false;
        draw
    }

    pub fn animating(&self) -> bool {
        self.screen_moving || !self.turn.is_settled() || !self.island_w.is_settled()
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
            self.turn_pictures = None;
        }
        g.canvas.px.copy_from_slice(&self.background);
        g.pager = None;
        // A title runs up to the island, however wide the island is just now.
        g.title_room = (W - self.island_w.value().max(g.island_h())) * 0.5 - PAD - 8.0;

        // Screens: during a page turn the arriving and leaving ones slide together.
        let turn = self.turn.value();
        let depth = self.stack.len();
        if turn.abs() > 0.002 {
            // Take the two pictures on the first frame of the turn.
            if self.turn_pictures.is_none() {
                let picture = |g: &mut Gfx, screen: Option<&mut Box<dyn Screen>>, home: bool| {
                    g.canvas.px.copy_from_slice(&self.background);
                    g.pager = None;
                    g.ox = 0.0;
                    if let Some(screen) = screen {
                        Self::draw_screen(g, screen.as_mut(), &self.robot, home);
                    }
                    (g.canvas.px.clone(), g.pager.take().map(|(pages, current, _)| (pages, current)))
                };
                self.turn_pictures = Some(if self.forward {
                    let under = if depth >= 2 { picture(g, self.stack.get_mut(depth - 2), depth == 2) } else { picture(g, None, false) };
                    [under, picture(g, self.stack.last_mut(), false)]
                } else {
                    [picture(g, self.stack.last_mut(), depth == 1), picture(g, self.leaving.as_mut(), false)]
                });
                g.canvas.px.copy_from_slice(&self.background);
            }
            // The one underneath drifts a third as far as the one on top travels.
            let (under_x, over_x) = if self.forward { (-(1.0 - turn) * W * 0.3, turn * W) } else { (-turn * W * 0.3, (1.0 - turn) * W) };
            if let Some([under, over]) = &self.turn_pictures {
                for ((pixels, dots), x) in [(under, under_x), (over, over_x)] {
                    blit_shifted(&mut g.canvas.px, pixels, W as usize, H as usize, x.round() as i32);
                    if let Some((pages, current)) = dots {
                        g.pager = Some((*pages, *current, x.round()));
                    }
                }
            }
        } else {
            self.turn_pictures = None;
            g.ox = 0.0;
            Self::draw_screen(g, self.stack[depth - 1].as_mut(), &self.robot, depth == 1);
        }
        g.ox = 0.0;

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
        if (want - self.island_w.target()).abs() > 0.5 {
            // Growing arrives on the release spring; shrinking leaves on smooth.
            let growing = want > self.island_w.target();
            self.island_w.set_with(want, if growing { role::release() } else { role::smooth() });
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

        // What sits on glass is not glass.
        g.canvas.set_clip(island.inset(2.0));
        let dot = if icon { t.signal } else if linked { t.ice } else { t.faint };
        g.canvas.circle(island.x + 12.0, island.cy(), 3.0, dot, 1.0);
        g.text_at(island.x + 22.0, island.cy(), Align::Left, &label, caption, t.ink);
        g.canvas.clear_clip();
    }

    fn draw_screen(g: &mut Gfx, screen: &mut dyn Screen, robot: &Robot, home: bool) {
        let t = g.tokens;
        let ty = g.ty;
        // The title shares the island's line; it gives way to a wide island.
        let bar_cy = 4.0 + g.island_h() * 0.5;
        let room = g.title_room;
        g.text_fit(PAD + 2.0, bar_cy, screen.title(), ty.title, t.ink, room);
        if !home {
            g.text_at(W - PAD - 2.0, bar_cy, Align::Right, "esc", ty.caption, t.dim);
        }
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
