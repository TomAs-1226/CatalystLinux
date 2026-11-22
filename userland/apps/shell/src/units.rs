//! Units: one value in every unit of its kind at once. Pick a kind (length,
//! torque, speed…), type a number against the unit it is in, and every other
//! unit's row shows the same amount; moving to another row takes its value
//! as the one to change. The units are the ones a robot is measured in: SI
//! beside inches, pounds, in·lb, ft/s, psi and rpm.

use bezel::{Align, Rect};

use crate::gfx::{Gfx, ListState, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

/// A unit: its name, and how many of the kind's base unit one of it is (and
/// an offset, for temperatures: base = value × factor + offset).
#[derive(Clone, Copy, Debug)]
pub struct Unit {
    pub name: &'static str,
    pub factor: f64,
    pub offset: f64,
}

const fn u(name: &'static str, factor: f64) -> Unit {
    Unit { name, factor, offset: 0.0 }
}

/// A kind of quantity, its units in the order shown.
pub struct Kind {
    pub name: &'static str,
    pub units: &'static [Unit],
}

const INCH: f64 = 0.0254;
const POUND: f64 = 0.453_592_37;
const POUND_FORCE: f64 = 4.448_221_615_260_5;
const G: f64 = 9.806_65;

pub const KINDS: &[Kind] = &[
    Kind { name: "Length", units: &[u("m", 1.0), u("cm", 0.01), u("mm", 0.001), u("km", 1000.0), u("in", INCH), u("ft", 12.0 * INCH), u("yd", 36.0 * INCH), u("mi", 63_360.0 * INCH)] },
    Kind { name: "Mass", units: &[u("kg", 1.0), u("g", 0.001), u("lb", POUND), u("oz", POUND / 16.0)] },
    Kind { name: "Force", units: &[u("N", 1.0), u("lbf", POUND_FORCE), u("kgf", G), u("ozf", POUND_FORCE / 16.0)] },
    Kind { name: "Torque", units: &[u("N·m", 1.0), u("in·lb", POUND_FORCE * INCH), u("ft·lb", POUND_FORCE * 12.0 * INCH), u("in·oz", POUND_FORCE * INCH / 16.0), u("kgf·cm", G * 0.01), u("mN·m", 0.001)] },
    Kind { name: "Speed", units: &[u("m/s", 1.0), u("ft/s", 12.0 * INCH), u("in/s", INCH), u("km/h", 1000.0 / 3600.0), u("mph", 63_360.0 * INCH / 3600.0)] },
    Kind {
        name: "Turning speed",
        units: &[u("rpm", std::f64::consts::TAU / 60.0), u("rad/s", 1.0), u("deg/s", std::f64::consts::PI / 180.0), u("rps", std::f64::consts::TAU)],
    },
    Kind { name: "Angle", units: &[u("deg", std::f64::consts::PI / 180.0), u("rad", 1.0), u("turns", std::f64::consts::TAU), u("arcmin", std::f64::consts::PI / 10_800.0)] },
    Kind { name: "Acceleration", units: &[u("m/s²", 1.0), u("ft/s²", 12.0 * INCH), u("g", G)] },
    Kind { name: "Power", units: &[u("W", 1.0), u("kW", 1000.0), u("hp", 745.699_871_582_270_2), u("mW", 0.001)] },
    Kind { name: "Energy", units: &[u("J", 1.0), u("kJ", 1000.0), u("Wh", 3600.0), u("kWh", 3_600_000.0), u("cal", 4.184), u("kcal", 4184.0)] },
    Kind { name: "Charge", units: &[u("mAh", 3.6), u("Ah", 3600.0), u("C", 1.0)] },
    Kind { name: "Current", units: &[u("A", 1.0), u("mA", 0.001), u("µA", 0.000_001)] },
    Kind { name: "Voltage", units: &[u("V", 1.0), u("mV", 0.001), u("kV", 1000.0)] },
    Kind { name: "Pressure", units: &[u("psi", POUND_FORCE / (INCH * INCH)), u("kPa", 1000.0), u("bar", 100_000.0), u("atm", 101_325.0), u("Pa", 1.0)] },
    Kind {
        name: "Temperature",
        units: &[Unit { name: "°C", factor: 1.0, offset: 0.0 }, Unit { name: "°F", factor: 5.0 / 9.0, offset: -32.0 * 5.0 / 9.0 }, Unit { name: "K", factor: 1.0, offset: -273.15 }],
    },
    Kind { name: "Data", units: &[u("B", 1.0), u("KB", 1000.0), u("MB", 1e6), u("GB", 1e9), u("KiB", 1024.0), u("MiB", 1_048_576.0), u("GiB", 1_073_741_824.0), u("bit", 0.125)] },
    Kind { name: "Time", units: &[u("s", 1.0), u("ms", 0.001), u("min", 60.0), u("h", 3600.0), u("day", 86_400.0)] },
];

/// `value` in `from` as an amount of `to`.
pub fn convert(value: f64, from: &Unit, to: &Unit) -> f64 {
    let base = value * from.factor + from.offset;
    (base - to.offset) / to.factor
}

/// An amount as the rows show it: up to seven significant figures, no
/// trailing zeros, powers of ten when very large or small.
pub fn amount(x: f64) -> String {
    figures(x, 7)
}

/// `x` to `sig` significant figures, as above.
fn figures(x: f64, sig: i32) -> String {
    if !x.is_finite() {
        return "–".into();
    }
    if x == 0.0 {
        return "0".into();
    }
    let magnitude = x.abs().log10().floor() as i32;
    if !(-4..9).contains(&magnitude) {
        let s = format!("{x:.prec$e}", prec = (sig - 2).max(1) as usize);
        let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
        return format!("{}e{e}", m.trim_end_matches('0').trim_end_matches('.'));
    }
    let decimals = (sig - 1 - magnitude).clamp(0, 12) as usize;
    let s = format!("{x:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    }
}

// ----------------------------------------------------------------- screens ---

/// The kinds, to choose one.
pub struct UnitsApp {
    list: ListState,
}

impl UnitsApp {
    pub fn new() -> Self {
        UnitsApp { list: ListState::default() }
    }
}

impl Screen for UnitsApp {
    fn title(&self) -> &str {
        "Units"
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Back => Nav::Pop,
            Key::Enter | Key::Right => Nav::Push(Box::new(Converter::new(self.list.focus))),
            other => {
                crate::screens::list_key(&mut self.list, other, KINDS.len());
                Nav::Stay
            }
        }
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        for (i, y) in self.list.layout(g, KINDS.len(), top, row_h, 3.0, bottom) {
            let kind = &KINDS[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            let names: Vec<&str> = kind.units.iter().take(3).map(|u| u.name).collect();
            g.row(r, kind.name, &names.join(" · "), t.dim, i == self.list.focus);
        }
        self.list.scrollbar(g, KINDS.len(), top, bottom);
        g.hint("enter: convert");
    }
}

/// One kind: a value in one unit, shown in all of them.
pub struct Converter {
    kind: usize,
    /// The row being typed into, and what is typed there.
    from: usize,
    typed: String,
    list: ListState,
}

impl Converter {
    fn new(kind: usize) -> Self {
        Converter { kind, from: 0, typed: "1".into(), list: ListState::default() }
    }

    /// A kind open on an amount already typed against one of its units (from search).
    pub fn at(kind: usize, from: usize, typed: &str) -> Self {
        let kind = kind.min(KINDS.len() - 1);
        let from = from.min(KINDS[kind].units.len() - 1);
        let mut list = ListState::default();
        list.focus = from;
        Converter { kind, from, typed: typed.chars().take(18).collect(), list }
    }

    fn value(&self) -> f64 {
        self.typed.replace(',', ".").parse::<f64>().unwrap_or(0.0)
    }

    /// Typing moves to `to`: it takes that row's value as its own.
    fn take(&mut self, to: usize) {
        let units = KINDS[self.kind].units;
        if to == self.from || to >= units.len() {
            return;
        }
        // Carried with a few more figures than shown, so going back and forth
        // does not wear the amount down.
        self.typed = figures(convert(self.value(), &units[self.from], &units[to]), 10);
        self.from = to;
    }
}

impl Screen for Converter {
    fn title(&self) -> &str {
        KINDS[self.kind].name
    }

    fn wants_text(&self) -> bool {
        true
    }

    /// Copy: the amount typed, with its unit (clipboard.rs).
    fn copy(&mut self, _robot: &Robot) -> Option<String> {
        let unit = KINDS[self.kind].units.get(self.from)?;
        Some(format!("{} {}", self.typed, unit.name))
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        let len = KINDS[self.kind].units.len();
        match key {
            Key::Back | Key::Leave => return Nav::Pop,
            Key::Up => {
                let to = self.from.saturating_sub(1);
                self.take(to);
            }
            Key::Down => {
                let to = (self.from + 1).min(len - 1);
                self.take(to);
            }
            Key::Backspace => {
                self.typed.pop();
            }
            Key::Char('-') if self.typed.is_empty() || self.typed == "0" => self.typed = "-".into(),
            Key::Char('-') => {
                if let Some(rest) = self.typed.strip_prefix('-') {
                    self.typed = rest.to_owned();
                } else {
                    self.typed.insert(0, '-');
                }
            }
            Key::Char(c) if (c.is_ascii_digit() || c == '.' || c == ',') && self.typed.len() < 18 => {
                if self.typed == "0" && c.is_ascii_digit() {
                    self.typed.clear();
                }
                if (c == '.' || c == ',') && self.typed.contains(['.', ',']) {
                    return Nav::Stay;
                }
                self.typed.push(c);
            }
            Key::Char('e') if !self.typed.contains('e') && !self.typed.is_empty() => self.typed.push('e'),
            Key::Enter => {}
            _ => {}
        }
        self.list.focus = self.from;
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let units = KINDS[self.kind].units;
        let (top, row_h, bottom) = (g.top(), g.row_h(), g.above_hint());
        self.list.focus = self.from;
        let value = self.value();
        for (i, y) in self.list.layout(g, units.len(), top, row_h, 3.0, bottom) {
            let unit = &units[i];
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, row_h);
            if i == self.from {
                g.row(r, unit.name, "", t.ink, true);
                let cy = r.cy();
                let shown = if self.typed.is_empty() { "0".to_owned() } else { self.typed.clone() };
                let right = r.right() - 12.0;
                g.text_at(right - 4.0, cy, Align::Right, &shown, ty.label, t.ink);
                g.surface(Rect::new(right - 2.0, cy - ty.label.size * 0.5, 1.5, ty.label.size), 0.5, t.signal);
            } else {
                let v = convert(value, &units[self.from], unit);
                g.row(r, unit.name, &amount(v), t.dim, false);
            }
        }
        self.list.scrollbar(g, units.len(), top, bottom);
        g.hint_any(&["type a number · ↑↓: another unit · -: sign", "↑↓: another unit · -: sign"]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(kind: &str, name: &str) -> Unit {
        *KINDS.iter().find(|k| k.name == kind).unwrap().units.iter().find(|u| u.name == name).unwrap()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * b.abs().max(1.0)
    }

    #[test]
    fn robot_units_convert_both_ways() {
        assert!(close(convert(1.0, &unit("Length", "in"), &unit("Length", "mm")), 25.4));
        assert!(close(convert(1.0, &unit("Torque", "N·m"), &unit("Torque", "in·lb")), 8.850_745_791_327_183));
        assert!(close(convert(10.0, &unit("Speed", "ft/s"), &unit("Speed", "m/s")), 3.048));
        assert!(close(convert(6000.0, &unit("Turning speed", "rpm"), &unit("Turning speed", "rad/s")), 628.318_530_717_958_6));
        assert!(close(convert(120.0, &unit("Pressure", "psi"), &unit("Pressure", "kPa")), 827.370_875_7));
        assert!(close(convert(1.0, &unit("Mass", "lb"), &unit("Mass", "kg")), POUND));
        assert!(close(convert(1.0, &unit("Force", "lbf"), &unit("Force", "N")), POUND_FORCE));
        for kind in KINDS {
            for a in kind.units {
                for b in kind.units {
                    let there = convert(3.7, a, b);
                    assert!(close(convert(there, b, a), 3.7), "{} {} ↔ {}", kind.name, a.name, b.name);
                }
            }
        }
    }

    #[test]
    fn temperatures_keep_their_offsets() {
        let (c, f, k) = (unit("Temperature", "°C"), unit("Temperature", "°F"), unit("Temperature", "K"));
        assert!(close(convert(100.0, &c, &f), 212.0));
        assert!(close(convert(32.0, &f, &c), 0.0));
        assert!(close(convert(0.0, &c, &k), 273.15));
        assert!(close(convert(-40.0, &f, &c), -40.0));
    }

    #[test]
    fn amounts_read_cleanly() {
        assert_eq!(amount(25.4), "25.4");
        assert_eq!(amount(8.850_745_791), "8.850746");
        assert_eq!(amount(1_000_000.0), "1000000");
        assert_eq!(amount(0.000_012), "1.2e-5");
        assert_eq!(amount(3.0e12), "3e12");
        assert_eq!(amount(0.0), "0");
    }

    #[test]
    fn moving_to_another_row_carries_the_amount() {
        let mut c = Converter::new(KINDS.iter().position(|k| k.name == "Length").unwrap());
        c.typed = "1".into();
        let inch = KINDS[c.kind].units.iter().position(|u| u.name == "in").unwrap();
        c.take(inch);
        assert_eq!(c.from, inch);
        assert!(close(c.value(), 39.370_078_74));
    }
}
