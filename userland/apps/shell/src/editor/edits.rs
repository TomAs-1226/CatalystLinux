//! What an edit is: the adjustments, the look, the geometry and the marks,
//! kept beside the photo in a small text file (`.edits/IMG_….jpg.edit`), so
//! the photo itself is never changed and an edit can be taken up again.

use std::path::{Path, PathBuf};

use crate::filters::Look;

/// The adjustments, in the panel's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Adjust {
    Exposure,
    Contrast,
    Highlights,
    Shadows,
    Saturation,
    Warmth,
    Tint,
    Vibrance,
    Sharpness,
    Vignette,
}

pub const ADJUSTS: [Adjust; 10] = [
    Adjust::Exposure,
    Adjust::Contrast,
    Adjust::Highlights,
    Adjust::Shadows,
    Adjust::Saturation,
    Adjust::Warmth,
    Adjust::Tint,
    Adjust::Vibrance,
    Adjust::Sharpness,
    Adjust::Vignette,
];

impl Adjust {
    pub fn name(self) -> &'static str {
        match self {
            Adjust::Exposure => "exposure",
            Adjust::Contrast => "contrast",
            Adjust::Highlights => "highlights",
            Adjust::Shadows => "shadows",
            Adjust::Saturation => "saturation",
            Adjust::Warmth => "warmth",
            Adjust::Tint => "tint",
            Adjust::Vibrance => "vibrance",
            Adjust::Sharpness => "sharpness",
            Adjust::Vignette => "vignette",
        }
    }

    pub fn index(self) -> usize {
        ADJUSTS.iter().position(|a| *a == self).unwrap_or(0)
    }

    /// Its least value: sharpness only adds, the rest go both ways.
    pub fn min(self) -> i32 {
        if self == Adjust::Sharpness {
            0
        } else {
            -100
        }
    }

    /// Its value as the row says it: exposure in stops (50 a stop), the
    /// rest as they are, signed.
    pub fn words(self, v: i32) -> String {
        match self {
            Adjust::Exposure if v == 0 => "0.0".into(),
            Adjust::Exposure => format!("{:+.1}", v as f32 / 50.0).replace('-', "−"),
            Adjust::Sharpness => v.to_string(),
            _ if v == 0 => "0".into(),
            _ => format!("{v:+}").replace('-', "−"),
        }
    }
}

/// The crop's shapes. Each fixed one follows the picture's way round (4:3
/// on a picture turned upright is 3:4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Aspect {
    #[default]
    Free,
    FourThree,
    ThreeTwo,
    SixteenNine,
    Square,
}

pub const ASPECTS: [Aspect; 5] = [Aspect::Free, Aspect::FourThree, Aspect::ThreeTwo, Aspect::SixteenNine, Aspect::Square];

impl Aspect {
    pub fn name(self) -> &'static str {
        match self {
            Aspect::Free => "free",
            Aspect::FourThree => "4:3",
            Aspect::ThreeTwo => "3:2",
            Aspect::SixteenNine => "16:9",
            Aspect::Square => "1:1",
        }
    }

    /// Its long side over its short one; none for free.
    pub fn ratio(self) -> Option<f32> {
        match self {
            Aspect::Free => None,
            Aspect::FourThree => Some(4.0 / 3.0),
            Aspect::ThreeTwo => Some(1.5),
            Aspect::SixteenNine => Some(16.0 / 9.0),
            Aspect::Square => Some(1.0),
        }
    }

    fn from_name(name: &str) -> Aspect {
        ASPECTS.iter().copied().find(|a| a.name() == name).unwrap_or_default()
    }
}

/// A mark's colour: the signal colour, or ice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    Signal,
    Ice,
}

impl Ink {
    pub fn name(self) -> &'static str {
        match self {
            Ink::Signal => "signal",
            Ink::Ice => "ice",
        }
    }
}

/// What a mark is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Arrow,
    Box,
    Circle,
    Text,
}

pub const SHAPES: [Shape; 4] = [Shape::Arrow, Shape::Box, Shape::Circle, Shape::Text];

impl Shape {
    pub fn name(self) -> &'static str {
        match self {
            Shape::Arrow => "arrow",
            Shape::Box => "box",
            Shape::Circle => "circle",
            Shape::Text => "text",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Shape::Arrow => "north_east",
            Shape::Box => "crop_square",
            Shape::Circle => "radio_button_unchecked",
            Shape::Text => "title",
        }
    }
}

/// A mark over the photo, for documenting a robot: an arrow from `a` to
/// `b`, a box or a circle (an ellipse in the box) with `a` and `b` at its
/// corners, or a label at `a`. Places are 0 to 1 of the edited picture (after
/// its crop), so a mark stays where it was put at any size.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub shape: Shape,
    pub ink: Ink,
    pub a: (f32, f32),
    pub b: (f32, f32),
    pub text: String,
}

/// Every edit to one photo.
#[derive(Clone, Debug, PartialEq)]
pub struct Edits {
    /// The adjustments' values, in [`ADJUSTS`]' order (−100 to 100;
    /// sharpness 0 to 100).
    pub values: [i32; 10],
    pub look: Look,
    /// Tint's colour (degrees), as the Camera's.
    pub hue: f32,
    /// Quarter turns clockwise (0 to 3), then flips, then the straightening
    /// (degrees, ±15, clockwise), then the crop.
    pub turns: u8,
    pub flip_h: bool,
    pub flip_v: bool,
    pub straighten: f32,
    pub aspect: Aspect,
    /// The crop: x, y, width and height, 0 to 1 of the straightened picture.
    pub crop: [f32; 4],
    pub marks: Vec<Mark>,
}

impl Default for Edits {
    fn default() -> Self {
        Edits { values: [0; 10], look: Look::Original, hue: 32.0, turns: 0, flip_h: false, flip_v: false, straighten: 0.0, aspect: Aspect::Free, crop: WHOLE, marks: Vec::new() }
    }
}

/// The whole picture, as a crop.
pub const WHOLE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

impl Edits {
    pub fn get(&self, a: Adjust) -> i32 {
        self.values[a.index()]
    }

    pub fn set(&mut self, a: Adjust, v: i32) {
        self.values[a.index()] = v.clamp(a.min(), 100);
    }

    /// Nothing changed: the photo as it is.
    pub fn is_plain(&self) -> bool {
        // Tint's colour counts only under Tint, which is not plain anyway.
        let plain = Edits::default();
        Edits { hue: plain.hue, ..self.clone() } == plain
    }

    /// The geometry alone (what the working picture is turned and cut by).
    pub fn geometry(&self) -> (u8, bool, bool, i32, [i32; 4]) {
        let q = |v: f32| (v * 10_000.0).round() as i32;
        (self.turns, self.flip_h, self.flip_v, (self.straighten * 100.0).round() as i32, self.crop.map(q))
    }

    /// The sidecar's text: a line a setting, only those not at rest.
    pub fn to_text(&self) -> String {
        let mut out = String::from("catalyst-edit 1\n");
        for a in ADJUSTS {
            if self.get(a) != 0 {
                out.push_str(&format!("{}={}\n", a.name(), self.get(a)));
            }
        }
        if self.look != Look::Original {
            out.push_str(&format!("look={}\n", self.look.id()));
            if self.look == Look::Tint {
                out.push_str(&format!("hue={:.0}\n", self.hue));
            }
        }
        if self.turns % 4 != 0 {
            out.push_str(&format!("turns={}\n", self.turns % 4));
        }
        if self.flip_h || self.flip_v {
            out.push_str(&format!("flip={}{}\n", if self.flip_h { "h" } else { "" }, if self.flip_v { "v" } else { "" }));
        }
        if self.straighten != 0.0 {
            out.push_str(&format!("straighten={:.2}\n", self.straighten));
        }
        if self.aspect != Aspect::Free {
            out.push_str(&format!("aspect={}\n", self.aspect.name()));
        }
        if self.crop != WHOLE {
            let [x, y, w, h] = self.crop;
            out.push_str(&format!("crop={x:.4},{y:.4},{w:.4},{h:.4}\n"));
        }
        for m in &self.marks {
            let text = m.text.replace(['\n', '\r'], " ");
            out.push_str(&format!("mark={},{},{:.4},{:.4},{:.4},{:.4},{text}\n", m.shape.name(), m.ink.name(), m.a.0, m.a.1, m.b.0, m.b.1));
        }
        out
    }

    /// An edit read back from its text; what it does not know is left at rest.
    pub fn from_text(text: &str) -> Edits {
        let mut e = Edits::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let num = |s: &str| s.trim().parse::<f32>().ok().filter(|n| n.is_finite());
            if let Some(a) = ADJUSTS.iter().copied().find(|a| a.name() == k) {
                if let Some(n) = num(v) {
                    e.set(a, n.round() as i32);
                }
                continue;
            }
            match k {
                "look" => e.look = Look::from_id(v.trim()).unwrap_or_default(),
                "hue" => e.hue = num(v).unwrap_or(e.hue).rem_euclid(360.0),
                "turns" => e.turns = (num(v).unwrap_or(0.0) as i32).rem_euclid(4) as u8,
                "flip" => {
                    e.flip_h = v.contains('h');
                    e.flip_v = v.contains('v');
                }
                "straighten" => e.straighten = num(v).unwrap_or(0.0).clamp(-15.0, 15.0),
                "aspect" => e.aspect = Aspect::from_name(v.trim()),
                "crop" => {
                    let n: Vec<f32> = v.split(',').filter_map(num).collect();
                    if let [x, y, w, h] = n[..] {
                        e.crop = clamp_crop([x, y, w, h]);
                    }
                }
                "mark" => {
                    let parts: Vec<&str> = v.splitn(7, ',').collect();
                    if parts.len() < 6 {
                        continue;
                    }
                    let Some(shape) = SHAPES.iter().copied().find(|s| s.name() == parts[0]) else { continue };
                    let ink = if parts[1] == "ice" { Ink::Ice } else { Ink::Signal };
                    let n: Vec<f32> = parts[2..6].iter().filter_map(|s| num(s)).map(|f| f.clamp(0.0, 1.0)).collect();
                    if n.len() == 4 {
                        e.marks.push(Mark { shape, ink, a: (n[0], n[1]), b: (n[2], n[3]), text: parts.get(6).unwrap_or(&"").to_string() });
                    }
                }
                _ => {}
            }
        }
        e
    }
}

/// A crop kept inside the picture and at least a twentieth of it each way.
pub fn clamp_crop([x, y, w, h]: [f32; 4]) -> [f32; 4] {
    let (w, h) = (w.clamp(0.05, 1.0), h.clamp(0.05, 1.0));
    [x.clamp(0.0, 1.0 - w), y.clamp(0.0, 1.0 - h), w, h]
}

/// Where a photo's edit is kept: `.edits` beside it (hidden, as the
/// thumbnails are), named for the photo.
pub fn sidecar(photo: &Path) -> PathBuf {
    let name = photo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    photo.parent().unwrap_or(Path::new(".")).join(".edits").join(format!("{name}.edit"))
}

/// A photo's edit, or none yet.
pub fn load(photo: &Path) -> Edits {
    std::fs::read_to_string(sidecar(photo)).map(|t| Edits::from_text(&t)).unwrap_or_default()
}

/// A photo's edit kept (and gone again once it is back to plain).
pub fn save(photo: &Path, edits: &Edits) -> std::io::Result<()> {
    let file = sidecar(photo);
    if edits.is_plain() {
        return match std::fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::sys::write_atomic(&file, edits.to_text().as_bytes())
}

/// Whether a photo has an edit kept.
pub fn has_edit(photo: &Path) -> bool {
    sidecar(photo).is_file()
}
