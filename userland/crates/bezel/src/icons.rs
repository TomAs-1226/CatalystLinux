//! Material Symbols Rounded, rasterized from their SVG outlines at whatever
//! size a control asks for. A selected item's icon is the filled variant.

use std::collections::HashMap;

use swash::zeno::{Mask, Transform};

use crate::canvas::Canvas;
use crate::color::Color;

include!(concat!(env!("OUT_DIR"), "/icons.rs"));

/// Icons added while running: an app's own symbol, read from the `icon.svg`
/// it carries, under a name of its own (`app:sudoku`). Drawn exactly as the
/// built-in ones are.
static REGISTERED: std::sync::RwLock<Vec<(String, String)>> = std::sync::RwLock::new(Vec::new());

/// The longest path an added icon may have: a Material Symbol is a few
/// hundred bytes to a few kilobytes.
const MAX_PATH: usize = 16 * 1024;

/// The path data of a symbol drawn in Material Symbols' 960-unit box (as
/// fonts.gstatic.com serves them), all its paths together. `None` for an
/// SVG drawn in any other box, with no path, or too large.
pub fn svg_path(svg: &str) -> Option<String> {
    if !svg.contains("viewBox=\"0 -960 960 960\"") {
        return None;
    }
    let mut d = String::new();
    let mut rest = svg;
    while let Some(at) = rest.find(" d=\"") {
        rest = &rest[at + 4..];
        let end = rest.find('"')?;
        d.push_str(&rest[..end]);
        d.push(' ');
        rest = &rest[end..];
    }
    let fine = !d.trim().is_empty() && d.len() <= MAX_PATH && d.chars().all(|c| c.is_ascii_alphanumeric() || " .,-+".contains(c));
    fine.then_some(d)
}

/// Add an icon under `name` from its path data (see [`svg_path`]); a name
/// already added is replaced. A built-in symbol's name cannot be taken.
pub fn register(name: &str, path: String) -> bool {
    if Icons::builtin(name) || path.len() > MAX_PATH {
        return false;
    }
    let Ok(mut added) = REGISTERED.write() else { return false };
    if let Some(entry) = added.iter_mut().find(|(n, _)| n == name) {
        if entry.1 == path {
            return true;
        }
        // Changed (an app updated): the old one is left unnamed rather than
        // overwritten, so no picture of it kept by an `Icons` is used again.
        entry.0.clear();
    }
    added.push((name.to_owned(), path));
    true
}

struct Raster {
    left: i32,
    top: i32,
    w: i32,
    h: i32,
    data: Vec<u8>,
}

#[derive(Default)]
pub struct Icons {
    cache: HashMap<(usize, u16), Raster>,
}

impl Icons {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether an icon of this name was built in, or has been added.
    pub fn has(name: &str) -> bool {
        Self::builtin(name) || REGISTERED.read().is_ok_and(|added| added.iter().any(|(n, _)| n == name))
    }

    /// Whether an icon of this name was built in.
    pub fn builtin(name: &str) -> bool {
        ICONS.binary_search_by(|(n, _)| (*n).cmp(name)).is_ok()
    }

    /// Draw `name` centred on (`cx`, `cy`) in a `size` px box. False when
    /// there is no icon of that name, and nothing was drawn.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, canvas: &mut Canvas, name: &str, filled: bool, cx: f32, cy: f32, size: f32, color: Color, alpha: f32) -> bool {
        // Built in by place in the table; added ones after them, by place in theirs.
        let lookup = |n: &str| {
            ICONS.binary_search_by(|(k, _)| (*k).cmp(n)).ok().or_else(|| REGISTERED.read().ok()?.iter().position(|(k, _)| k == n).map(|i| ICONS.len() + i))
        };
        let index = if filled { lookup(&format!("{name}_fill")).or_else(|| lookup(name)) } else { lookup(name) };
        let Some(index) = index else { return false };
        let path: std::borrow::Cow<str> = match ICONS.get(index) {
            Some((_, d)) => (*d).into(),
            None => match REGISTERED.read().ok().and_then(|added| added.get(index - ICONS.len()).map(|(_, d)| d.clone())) {
                Some(d) => d.into(),
                None => return false,
            },
        };

        let key = (index, (size * 4.0) as u16);
        let raster = self.cache.entry(key).or_insert_with(|| {
            // The symbols are drawn in a 960-unit box whose y runs −960..0.
            let s = size / 960.0;
            let (data, place) = Mask::new(path.as_ref())
                .transform(Some(Transform::translation(0.0, 960.0).then_scale(s, s)))
                .render();
            Raster { left: place.left, top: place.top, w: place.width as i32, h: place.height as i32, data }
        });
        let x = (cx - size * 0.5).round() as i32 + raster.left;
        let y = (cy - size * 0.5).round() as i32 + raster.top;
        canvas.mask(x, y, raster.w, raster.h, &raster.data, color, alpha);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_added_icon_draws_as_a_built_in_one_does() {
        // A square in Material Symbols' box, as an app would carry it.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" height="20" viewBox="0 -960 960 960" width="20"><path d="M200-160q-17 0-28.5-11.5T160-200v-560q0-17 11.5-28.5T200-800h560q17 0 28.5 11.5T800-760v560q0 17-11.5 28.5T760-160H200Z"/></svg>"#;
        let d = svg_path(svg).expect("a 960-unit symbol");
        assert!(svg_path(&svg.replace("0 -960 960 960", "0 0 24 24")).is_none(), "another box is refused");
        assert!(svg_path(&svg.replace("q-17", "q<script>")).is_none(), "anything but path data is refused");
        assert!(!register("grid_on", d.clone()), "a built-in name cannot be taken");
        assert!(register("app:test-square", d.clone()));
        assert!(Icons::has("app:test-square") && !Icons::builtin("app:test-square"));
        let mut canvas = Canvas::new(40, 40);
        let mut icons = Icons::new();
        assert!(icons.draw(&mut canvas, "app:test-square", false, 20.0, 20.0, 24.0, Color::WHITE, 1.0));
        let lit = |c: &Canvas| c.px.iter().filter(|p| **p != 0).count();
        assert!(lit(&canvas) > 100, "drawn: {}", lit(&canvas));
        assert!(!icons.draw(&mut canvas, "app:not-there", false, 20.0, 20.0, 24.0, Color::WHITE, 1.0));
        // Replaced by a new version: the new picture, not the one kept.
        assert!(register("app:test-square", "M0-960h100v100h-100z".into()));
        let mut small = Canvas::new(40, 40);
        icons.draw(&mut small, "app:test-square", false, 20.0, 20.0, 24.0, Color::WHITE, 1.0);
        assert!(lit(&small) > 0 && lit(&small) < lit(&canvas), "{} then {}", lit(&canvas), lit(&small));
    }
}
