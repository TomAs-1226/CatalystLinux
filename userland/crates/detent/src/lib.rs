//! Detent's spring model, ported from the Detent engine.
//!
//! Every spring is the damped oscillator `m·x″ + c·x′ + k·(x − target) = 0`,
//! solved in closed form, so a value can be read at any time without stepping
//! and a retarget keeps the velocity it had.
//!
//! The constructors are Detent's vocabularies (`platforms`, *One model*); the
//! presets are the ones Bezel names by role.

/// A spring as mass, stiffness and damping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub mass: f32,
    pub stiffness: f32,
    pub damping: f32,
}

impl Spring {
    /// SwiftUI `Spring(duration:bounce:)`.
    pub fn duration_bounce(duration: f32, bounce: f32) -> Self {
        let tau = std::f32::consts::TAU;
        let k = (tau / duration).powi(2);
        let c = if bounce >= 0.0 {
            2.0 * tau * (1.0 - bounce) / duration
        } else {
            2.0 * tau / (duration * (1.0 + bounce))
        };
        Spring { mass: 1.0, stiffness: k, damping: c }
    }

    /// SwiftUI `.spring(response:dampingFraction:)`.
    pub fn response_damping(response: f32, damping_fraction: f32) -> Self {
        let tau = std::f32::consts::TAU;
        Spring {
            mass: 1.0,
            stiffness: (tau / response).powi(2),
            damping: 2.0 * tau * damping_fraction / response,
        }
    }

    /// Compose and Material 3: `spring(dampingRatio, stiffness)`, mass 1.
    pub fn stiffness_ratio(stiffness: f32, damping_ratio: f32) -> Self {
        Self::mass_stiffness_ratio(1.0, stiffness, damping_ratio)
    }

    pub fn mass_stiffness_ratio(mass: f32, stiffness: f32, damping_ratio: f32) -> Self {
        Spring { mass, stiffness, damping: 2.0 * damping_ratio * (stiffness * mass).sqrt() }
    }

    /// Natural frequency ω₀ = √(k/m).
    pub fn omega0(&self) -> f32 {
        (self.stiffness / self.mass).sqrt()
    }

    /// Damping ratio ζ = c / (2√(k·m)).
    pub fn damping_ratio(&self) -> f32 {
        self.damping / (2.0 * (self.stiffness * self.mass).sqrt())
    }

    /// The same stiffness, critically damped: Bezel's reduced-motion rule.
    pub fn calm(&self) -> Self {
        if self.damping_ratio() < 1.0 {
            Self::mass_stiffness_ratio(self.mass, self.stiffness, 1.0)
        } else {
            *self
        }
    }

    /// Displacement from the target and velocity, `t` seconds after starting
    /// `x0` away from the target with velocity `v0`.
    pub fn solve(&self, x0: f32, v0: f32, t: f32) -> (f32, f32) {
        let w0 = self.omega0();
        let z = self.damping_ratio();
        if (z - 1.0).abs() < 1e-4 {
            let d = v0 + w0 * x0;
            let e = (-w0 * t).exp();
            ((x0 + d * t) * e, (d - w0 * (x0 + d * t)) * e)
        } else if z < 1.0 {
            let a = z * w0;
            let wd = w0 * (1.0 - z * z).sqrt();
            let c = (v0 + a * x0) / wd;
            let e = (-a * t).exp();
            let (s, co) = (wd * t).sin_cos();
            (e * (x0 * co + c * s), e * (v0 * co - (a * c + x0 * wd) * s))
        } else {
            let root = (z * z - 1.0).sqrt();
            let r1 = -w0 * (z - root);
            let r2 = -w0 * (z + root);
            let a = (v0 - r2 * x0) / (r1 - r2);
            let b = x0 - a;
            let (e1, e2) = ((r1 * t).exp(), (r2 * t).exp());
            (a * e1 + b * e2, a * r1 * e1 + b * r2 * e2)
        }
    }
}

/// The springs Bezel names by role (`bezel`, *Motion*).
pub mod role {
    use super::Spring;

    /// A value following a held input. `apple-swiftui-interactive-spring`.
    pub fn hold() -> Spring { Spring::response_damping(0.15, 0.86) }
    /// Things arriving: the island growing, chrome returning. `apple-swiftui-snappy`.
    pub fn release() -> Spring { Spring::duration_bounce(0.5, 0.15) }
    /// Things leaving. `apple-swiftui-smooth`.
    pub fn smooth() -> Spring { Spring::duration_bounce(0.5, 0.0) }
    /// A value travelling to where it was set. `apple-wwdc18-pip`.
    pub fn settle() -> Spring { Spring::response_damping(0.4, 1.0) }
    /// A dial landing on its detent. `apple-wwdc18-rotation`.
    pub fn detent() -> Spring { Spring::response_damping(0.4, 0.8) }
    /// `apple-swiftui-bouncy`.
    pub fn wobble() -> Spring { Spring::duration_bounce(0.5, 0.3) }
    /// Colour and glass strength; never bounces. `m3-default-effects`.
    pub fn effect() -> Spring { Spring::stiffness_ratio(1600.0, 1.0) }
    /// State layers. `m3-fast-effects`.
    pub fn layer() -> Spring { Spring::stiffness_ratio(3800.0, 1.0) }
    /// `m3-expressive-fast-spatial`.
    pub fn tick() -> Spring { Spring::stiffness_ratio(800.0, 0.6) }
    /// Page turns: Detent's paging pattern.
    pub fn page() -> Spring { Spring::duration_bounce(0.45, 0.12) }
    /// Content springing back from an edge. `flutter-ios-bounce`.
    pub fn edge() -> Spring { Spring::mass_stiffness_ratio(0.5, 100.0, 1.1) }
}

/// A value driven by a spring. Retargeting mid-flight keeps the velocity, so
/// motion is always interruptible.
#[derive(Clone, Copy, Debug)]
pub struct Animated {
    spring: Spring,
    target: f32,
    x0: f32,
    v0: f32,
    t: f32,
    value: f32,
    velocity: f32,
    rest: f32,
}

impl Animated {
    pub fn new(value: f32, spring: Spring) -> Self {
        Animated { spring, target: value, x0: 0.0, v0: 0.0, t: 0.0, value, velocity: 0.0, rest: 0.001 }
    }

    /// How close to the target, in the value's own units, counts as arrived.
    pub fn with_rest(mut self, rest: f32) -> Self {
        self.rest = rest;
        self
    }

    pub fn value(&self) -> f32 { self.value }
    pub fn velocity(&self) -> f32 { self.velocity }
    pub fn target(&self) -> f32 { self.target }

    /// Head for a new target from wherever the value is, at its current speed.
    pub fn set(&mut self, target: f32) {
        if target == self.target {
            return;
        }
        self.target = target;
        self.x0 = self.value - target;
        self.v0 = self.velocity;
        self.t = 0.0;
    }

    /// Head for a new target on a different spring.
    pub fn set_with(&mut self, target: f32, spring: Spring) {
        self.spring = spring;
        self.target = f32::NAN; // force a restart even for the same target
        self.set(target);
    }

    /// Land at once: what keyboard-driven changes do in Bezel.
    pub fn jump(&mut self, value: f32) {
        self.target = value;
        self.value = value;
        self.velocity = 0.0;
        self.x0 = 0.0;
        self.v0 = 0.0;
        self.t = 0.0;
    }

    /// Add an impulse without changing the target.
    pub fn nudge(&mut self, velocity: f32) {
        self.x0 = self.value - self.target;
        self.v0 = self.velocity + velocity;
        self.velocity = self.v0;
        self.t = 0.0;
    }

    pub fn is_settled(&self) -> bool {
        (self.value - self.target).abs() <= self.rest && self.velocity.abs() <= self.rest * 10.0
    }

    /// Advance by `dt` seconds. Returns true while still moving.
    pub fn tick(&mut self, dt: f32) -> bool {
        if self.x0 == 0.0 && self.v0 == 0.0 {
            return false;
        }
        self.t += dt;
        let (x, v) = self.spring.solve(self.x0, self.v0, self.t);
        self.value = self.target + x;
        self.velocity = v;
        if self.is_settled() {
            self.jump(self.target);
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, tol: f32) -> bool { (a - b).abs() <= tol }

    // The two conversions Detent's own suite checks against Apple's documentation.
    #[test]
    fn duration_bounce_matches_apple() {
        let s = Spring::duration_bounce(0.5, 0.3);
        assert!(close(s.stiffness, 157.9, 0.1), "{}", s.stiffness);
        assert!(close(s.damping, 17.6, 0.1), "{}", s.damping);
    }

    #[test]
    fn mass_stiffness_damping_round_trips() {
        // Spring(mass: 1, stiffness: 100, damping: 10) is duration 0.63, bounce 0.5.
        let s = Spring { mass: 1.0, stiffness: 100.0, damping: 10.0 };
        let duration = std::f32::consts::TAU / s.omega0();
        assert!(close(duration, 0.63, 0.005));
        assert!(close(1.0 - s.damping_ratio(), 0.5, 0.001));
    }

    #[test]
    fn solution_satisfies_the_equation_in_every_regime() {
        for spring in [role::release(), role::settle(), role::edge(), role::tick()] {
            let (x0, v0) = (1.0, -3.0);
            let (xa, va) = spring.solve(x0, v0, 0.0);
            assert!(close(xa, x0, 1e-4) && close(va, v0, 1e-3));
            // m·x″ + c·x′ + k·x = 0, with x″ from a central difference of x′.
            let (t, h) = (0.07, 1e-3);
            let (x, v) = spring.solve(x0, v0, t);
            let acc = (spring.solve(x0, v0, t + h).1 - spring.solve(x0, v0, t - h).1) / (2.0 * h);
            let residual = spring.mass * acc + spring.damping * v + spring.stiffness * x;
            assert!(residual.abs() < 0.5, "residual {residual} for {spring:?}");
        }
    }

    #[test]
    fn critically_damped_never_overshoots() {
        let s = role::settle();
        let mut t = 0.0;
        while t < 2.0 {
            assert!(s.solve(1.0, 0.0, t).0 >= -1e-5);
            t += 0.01;
        }
    }

    #[test]
    fn retarget_keeps_velocity_and_settles() {
        let mut a = Animated::new(0.0, role::release());
        a.set(100.0);
        for _ in 0..6 { a.tick(1.0 / 60.0); }
        let v = a.velocity();
        assert!(v > 0.0);
        a.set(-50.0);
        a.tick(1e-4);
        assert!(close(a.velocity(), v, v.abs() * 0.05 + 1.0));
        let mut frames = 0;
        while a.tick(1.0 / 60.0) { frames += 1; assert!(frames < 600); }
        assert_eq!(a.value(), -50.0);
    }

    #[test]
    fn calm_removes_overshoot() {
        assert!(role::wobble().damping_ratio() < 1.0);
        assert!(close(role::wobble().calm().damping_ratio(), 1.0, 1e-5));
    }
}
