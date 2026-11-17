//! Material Symbols Rounded, rasterized from their SVG outlines at whatever
//! size a control asks for. A selected item's icon is the filled variant.

use std::collections::HashMap;

use swash::zeno::{Mask, Transform};

use crate::canvas::Canvas;
use crate::color::Color;

include!(concat!(env!("OUT_DIR"), "/icons.rs"));

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

    /// Whether an icon of this name was built in.
    pub fn has(name: &str) -> bool {
        ICONS.binary_search_by(|(n, _)| (*n).cmp(name)).is_ok()
    }

    /// Draw `name` centred on (`cx`, `cy`) in a `size` px box.
    pub fn draw(&mut self, canvas: &mut Canvas, name: &str, filled: bool, cx: f32, cy: f32, size: f32, color: Color, alpha: f32) {
        let lookup = |n: &str| ICONS.binary_search_by(|(k, _)| (*k).cmp(n)).ok();
        let index = if filled { lookup(&format!("{name}_fill")).or_else(|| lookup(name)) } else { lookup(name) };
        let Some(index) = index else { return };

        let key = (index, (size * 4.0) as u16);
        let raster = self.cache.entry(key).or_insert_with(|| {
            // The symbols are drawn in a 960-unit box whose y runs −960..0.
            let s = size / 960.0;
            let (data, place) = Mask::new(ICONS[index].1)
                .transform(Some(Transform::translation(0.0, 960.0).then_scale(s, s)))
                .render();
            Raster { left: place.left, top: place.top, w: place.width as i32, h: place.height as i32, data }
        });
        let x = (cx - size * 0.5).round() as i32 + raster.left;
        let y = (cy - size * 0.5).round() as i32 + raster.top;
        canvas.mask(x, y, raster.w, raster.h, &raster.data, color, alpha);
    }
}
