//! The four universes, set up exactly as AstroBox's presets set them up:
//! the same constants, masses, places, speeds and colours.

use std::f64::consts::PI;

use crate::sim::{Config, Hue, Sim};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Universe {
    SolarSystem,
    FigureEight,
    ThreeBody,
    MilkyWay,
}

/// How fast time can run, slowest to fastest (AstroBox's scale).
pub const SPEEDS: [f64; 7] = [0.1, 0.25, 0.5, 1.0, 2.0, 4.0, 10.0];

/// ×1 on [`SPEEDS`].
pub const REAL_TIME: usize = 3;

impl Universe {
    pub const ALL: [Universe; 4] = [Universe::SolarSystem, Universe::FigureEight, Universe::ThreeBody, Universe::MilkyWay];

    pub fn name(self) -> &'static str {
        match self {
            Universe::SolarSystem => "Solar System",
            Universe::FigureEight => "Figure 8",
            Universe::ThreeBody => "3-Body Problem",
            Universe::MilkyWay => "Milky Way",
        }
    }

    /// One line about it, for the list.
    pub fn about(self) -> &'static str {
        match self {
            Universe::SolarSystem => "the sun and its eight planets",
            Universe::FigureEight => "three stars on one figure of eight",
            Universe::ThreeBody => "a close pair and a distant third",
            Universe::MilkyWay => "sixty-three stars round a core",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Universe::SolarSystem => "sunny",
            Universe::FigureEight => "animation",
            Universe::ThreeBody => "orbit",
            Universe::MilkyWay => "auto_awesome",
        }
    }

    /// How many bodies it starts with.
    pub fn bodies(self) -> usize {
        match self {
            Universe::SolarSystem => 9,
            Universe::FigureEight | Universe::ThreeBody => 3,
            Universe::MilkyWay => 64,
        }
    }

    /// The half-width and half-height of the universe the view opens on, in
    /// its own units: the inner planets, the whole figure of eight, the
    /// third star's whole circle, the galaxy's widest ring.
    pub fn frame(self) -> (f64, f64) {
        match self {
            Universe::SolarSystem => (1.75, 1.75),
            Universe::FigureEight => (1.15, 0.42),
            Universe::ThreeBody => (5.2, 5.0),
            Universe::MilkyWay => (118.0, 78.0),
        }
    }

    /// Where on [`SPEEDS`] time starts. The galaxy turns once in five
    /// minutes at its heart and in eighty at its edge, so it starts at ×4.
    pub fn start_speed(self) -> usize {
        match self {
            Universe::MilkyWay => REAL_TIME + 2,
            _ => REAL_TIME,
        }
    }

    pub fn load(self) -> Sim {
        match self {
            Universe::SolarSystem => solar_system(),
            Universe::FigureEight => figure_eight(),
            Universe::ThreeBody => three_body(),
            Universe::MilkyWay => milky_way(),
        }
    }
}

fn solar_system() -> Sim {
    let mut sim = Sim::new(Config { gravity: 0.0001, softening: 0.002, ..Config::default() });
    let g = sim.config.gravity;
    const SUN: f64 = 1000.0;
    sim.add("Sun", (0.0, 0.0), (0.0, 0.0), SUN, Hue::from_rgb(0xFFE66D), true);
    // Name, orbit (AU), angle (degrees), mass, colour.
    const PLANETS: [(&str, f64, f64, f64, u32); 8] = [
        ("Mercury", 0.39, 0.0, 0.20, 0xFFFFFF),
        ("Venus", 0.72, 55.0, 0.20, 0xFF8C42),
        ("Earth", 1.00, 115.0, 0.20, 0x2D8CFF),
        ("Mars", 1.52, 180.0, 0.20, 0xFF4D4D),
        ("Jupiter", 5.20, 235.0, 2.00, 0xFF8C42),
        ("Saturn", 9.58, 290.0, 0.60, 0xFFE66D),
        ("Uranus", 19.20, 25.0, 0.20, 0x4ECDC4),
        ("Neptune", 30.05, 155.0, 0.20, 0x2D8CFF),
    ];
    let (mut px, mut py) = (0.0, 0.0);
    for (name, orbit, degrees, mass, rgb) in PLANETS {
        let (sin, cos) = (degrees * PI / 180.0).sin_cos();
        let speed = (g * (SUN + mass) / orbit).sqrt();
        let vel = (-sin * speed, cos * speed);
        sim.add(name, (orbit * cos, orbit * sin), vel, mass, Hue::from_rgb(rgb), false);
        px += vel.0 * mass;
        py += vel.1 * mass;
    }
    // Into the barycentre's frame: no drift, centred on the centre of mass.
    let total = sim.total_mass();
    let centre = sim.centre_of_mass();
    sim.move_frame(centre, (px / total, py / total));
    sim
}

fn figure_eight() -> Sim {
    let mut sim = Sim::new(Config::default());
    sim.add("Astra", (-0.97000436, 0.24308753), (0.466203685, 0.432365730), 1.0, Hue::from_rgb(0xFFE66D), true);
    sim.add("Vega", (0.97000436, -0.24308753), (0.466203685, 0.432365730), 1.0, Hue::from_rgb(0xFF8C42), true);
    sim.add("Nova", (0.0, 0.0), (-0.93240737, -0.86473146), 1.0, Hue::from_rgb(0xFFFFFF), true);
    sim
}

/// A close binary with a lighter third far out, all on circular orbits
/// about their common centre.
fn three_body() -> Sim {
    let mut sim = Sim::new(Config { gravity: 0.2, softening: 0.003, ..Config::default() });
    let g = sim.config.gravity;
    const INNER_MASS: f64 = 1.0;
    const OUTER_MASS: f64 = 0.5;
    const INNER_SEPARATION: f64 = 1.0;
    const OUTER_SEPARATION: f64 = 6.0;
    const TOTAL: f64 = INNER_MASS * 2.0 + OUTER_MASS;
    let inner_centre_x = -OUTER_MASS * OUTER_SEPARATION / TOTAL;
    let outer_x = INNER_MASS * 2.0 * OUTER_SEPARATION / TOTAL;
    let inner_speed = (g * INNER_MASS / (2.0 * INNER_SEPARATION)).sqrt();
    let outer_angular_speed = (g * TOTAL / (OUTER_SEPARATION * OUTER_SEPARATION * OUTER_SEPARATION)).sqrt();
    let inner_centre_speed = outer_angular_speed * inner_centre_x;
    let outer_speed = outer_angular_speed * outer_x;
    sim.add("Aurelia", (inner_centre_x - INNER_SEPARATION * 0.5, 0.0), (0.0, inner_centre_speed - inner_speed), INNER_MASS, Hue::from_rgb(0xFFE66D), true);
    sim.add("Ember", (inner_centre_x + INNER_SEPARATION * 0.5, 0.0), (0.0, inner_centre_speed + inner_speed), INNER_MASS, Hue::from_rgb(0xFF8C42), true);
    sim.add("Ivory", (outer_x, 0.0), (0.0, outer_speed), OUTER_MASS, Hue::from_rgb(0xFFFFFF), true);
    sim
}

/// A star's colour in the galaxy: AstroBox's hash of its number into a
/// palette weighted toward white.
fn milky_way_star_color(star: u32) -> u32 {
    const COLORS: [u32; 16] = [
        0xFFFFFF, 0xFFFFFF, 0xFFFFFF, 0xFFFFFF, 0xFFFFFF, 0xFFFFFF, 0xFFE66D, 0xFFE66D, 0xFFE66D, 0xFF8C42, 0xFF8C42, 0x2D8CFF, 0x2D8CFF, 0x4ECDC4, 0xFFFFFF, 0xFF4D4D,
    ];
    let mut hash = star.wrapping_add(0x9E37_79B9);
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x7FEB_352D);
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(0x846C_A68B);
    hash ^= hash >> 16;
    COLORS[(hash % COLORS.len() as u32) as usize]
}

/// A heavy core with seven rings of nine stars on eccentric orbits, each
/// ring turned a little from the one inside it.
fn milky_way() -> Sim {
    let mut sim = Sim::new(Config {
        gravity: 0.001,
        softening: 0.10,
        step: 1.0 / 180.0,
        max_steps: 48,
        // AstroBox samples every 24 steps. At the whole-galaxy view that
        // leaves trails two or three pixels long, so Orbits samples six
        // times as seldom: the same 48 points reach six times as far back.
        trail_every: 144,
        ..Config::default()
    });
    let g = sim.config.gravity;
    const CORE: f64 = 1000.0;
    const STAR: f64 = 1.0;
    const PER_RING: usize = 9;
    const RINGS: usize = 7;
    sim.add("Sagittarius A", (0.0, 0.0), (0.0, 0.0), CORE, Hue::from_rgb(0xFFFFFF), true);
    let (mut px, mut py) = (0.0, 0.0);
    let mut number = 1u32;
    for ring in 0..RINGS {
        let semi_major = 13.0 + ring as f64 * 12.0;
        let eccentricity = 0.30 + 0.025 * (ring % 4) as f64;
        let semi_latus = semi_major * (1.0 - eccentricity * eccentricity);
        let enclosed = CORE + STAR * (ring * PER_RING) as f64 + STAR * PER_RING as f64 * 0.5;
        let velocity_scale = (g * enclosed / semi_latus).sqrt();
        let (apsis_sin, apsis_cos) = (ring as f64 * 0.08).sin_cos();
        let phase = ring as f64 * PI / PER_RING as f64;
        for slot in 0..PER_RING {
            let anomaly = phase + 2.0 * PI * slot as f64 / PER_RING as f64;
            let (sin, cos) = anomaly.sin_cos();
            let radius = semi_latus / (1.0 + eccentricity * cos);
            let (ox, oy) = (radius * cos, radius * sin);
            let (ovx, ovy) = (-velocity_scale * sin, velocity_scale * (eccentricity + cos));
            let pos = (ox * apsis_cos - oy * apsis_sin, ox * apsis_sin + oy * apsis_cos);
            let vel = (ovx * apsis_cos - ovy * apsis_sin, ovx * apsis_sin + ovy * apsis_cos);
            sim.add(format!("Star {number:02}"), pos, vel, STAR, Hue::from_rgb(milky_way_star_color(number)), true);
            number += 1;
            px += vel.0 * STAR;
            py += vel.1 * STAR;
        }
    }
    // The core moves so that the galaxy as a whole does not.
    sim.set_vel(0, (-px / CORE, -py / CORE));
    sim
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_universe_loads_with_its_bodies() {
        let counts: Vec<usize> = Universe::ALL.iter().map(|u| u.load().len()).collect();
        assert_eq!(counts, [9, 3, 3, 64]);
        for u in Universe::ALL {
            assert_eq!(u.load().len(), u.bodies(), "{}", u.name());
        }
    }

    #[test]
    fn every_universe_starts_still_as_a_whole() {
        for u in Universe::ALL {
            let sim = u.load();
            let (px, py) = sim.momentum();
            assert!(px.abs() < 1e-9 && py.abs() < 1e-9, "{}: momentum ({px}, {py})", u.name());
        }
        let (cx, cy) = Universe::SolarSystem.load().centre_of_mass();
        assert!(cx.abs() < 1e-12 && cy.abs() < 1e-12, "the solar system is centred on its barycentre");
    }

    #[test]
    fn the_presets_carry_astroboxs_constants() {
        let solar = Universe::SolarSystem.load();
        assert_eq!((solar.config.gravity, solar.config.softening), (0.0001, 0.002));
        assert_eq!(solar.mass(0), 1000.0);
        assert_eq!(solar.body(3).name, "Earth");
        let eight = Universe::FigureEight.load();
        assert_eq!((eight.config.gravity, eight.config.softening, eight.config.step), (1.0, 0.001, 1.0 / 240.0));
        let three = Universe::ThreeBody.load();
        assert_eq!((three.config.gravity, three.config.softening), (0.2, 0.003));
        let galaxy = Universe::MilkyWay.load();
        assert_eq!((galaxy.config.gravity, galaxy.config.softening, galaxy.config.step, galaxy.config.max_steps), (0.001, 0.1, 1.0 / 180.0, 48));
        assert_eq!(galaxy.mass(0), 1000.0);
        assert_eq!(galaxy.body(63).name, "Star 63");
        // The hash is AstroBox's: the first stars' colours.
        let first: Vec<Hue> = (1..=6).map(|i| galaxy.body(i).hue).collect();
        let expect: Vec<Hue> = (1..=6).map(|n| Hue::from_rgb(milky_way_star_color(n))).collect();
        assert_eq!(first, expect);
    }
}
