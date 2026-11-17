//! Bezel's colour roles. The values are `bezel_tokens` output for Bezel's own
//! palette; a seeded (Monet) palette would fill the same struct.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Color(pub u8, pub u8, pub u8);

impl Color {
    pub const WHITE: Color = Color(255, 255, 255);
    pub const BLACK: Color = Color(0, 0, 0);

    pub const fn hex(v: u32) -> Self {
        Color((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    pub const fn to_u32(self) -> u32 {
        (self.0 as u32) << 16 | (self.1 as u32) << 8 | self.2 as u32
    }

    pub fn mix(self, other: Color, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        let f = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t + 0.5) as u8;
        Color(f(self.0, other.0), f(self.1, other.1), f(self.2, other.2))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Dark,
    Light,
}

/// One tone's colour roles.
#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub tone: Tone,
    /// The screen behind everything.
    pub ground: Color,
    /// Tiles and chips at rest.
    pub surface1: Color,
    /// Tonal buttons; glass drawn solid in the calm version.
    pub surface2: Color,
    /// Stepper buttons and placeholders.
    pub surface3: Color,
    pub track: Color,
    pub meter: Color,
    /// Text and icons.
    pub ink: Color,
    /// Quiet text: labels, captions, values.
    pub dim: Color,
    /// Disabled text and inactive dots. Never for words people must read.
    pub faint: Color,
    /// The one signal colour: the primary action, heat, progress, the focus ring.
    pub signal: Color,
    pub on_signal: Color,
    /// Tonal "on" fills.
    pub ice: Color,
    pub on_ice: Color,
    pub amber: Color,
    pub on_amber: Color,
    pub amber_container: Color,
    pub amber_ink: Color,
}

impl Tokens {
    pub const fn dark() -> Self {
        Tokens {
            tone: Tone::Dark,
            ground: Color::hex(0x0b0c0e),
            surface1: Color::hex(0x16181b),
            surface2: Color::hex(0x1f2226),
            surface3: Color::hex(0x2a2e33),
            track: Color::hex(0x1f2226),
            meter: Color::hex(0x24272c),
            ink: Color::hex(0xeef0f2),
            dim: Color::hex(0x9aa1a8),
            faint: Color::hex(0x5d646b),
            signal: Color::hex(0xff5b1f),
            on_signal: Color::hex(0x1a0a02),
            ice: Color::hex(0xcfe0ee),
            on_ice: Color::hex(0x0e2233),
            amber: Color::hex(0xf4c542),
            on_amber: Color::hex(0x2a2000),
            amber_container: Color::hex(0x3b3218),
            amber_ink: Color::hex(0xf4c542),
        }
    }

    pub const fn light() -> Self {
        Tokens {
            tone: Tone::Light,
            ground: Color::hex(0xe8eaed),
            surface1: Color::hex(0xfbfbfc),
            surface2: Color::hex(0xeef0f3),
            surface3: Color::hex(0xe1e4e8),
            track: Color::hex(0xfbfbfc),
            meter: Color::hex(0xe4e7eb),
            ink: Color::hex(0x15181b),
            dim: Color::hex(0x5b636c),
            faint: Color::hex(0xa3aab2),
            signal: Color::hex(0xea5a1a),
            on_signal: Color::hex(0xffffff),
            ice: Color::hex(0xcde1f3),
            on_ice: Color::hex(0x0c2436),
            amber: Color::hex(0xffd24a),
            on_amber: Color::hex(0x2b2100),
            amber_container: Color::hex(0xf6e3a4),
            amber_ink: Color::hex(0x8a5f00),
        }
    }
}

/// The accent a palette grows from. Orange is Bezel's own; the others are
/// `bezel_tokens` grown from a seed the way Android's Monet does (the signal
/// keeps the seed's chroma, ice is its temperature complement, amber is
/// harmonized), so every one passes the same contrast checks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Accent {
    Orange,
    Blue,
    Green,
    Violet,
    Rose,
}

impl Accent {
    pub const ALL: [Accent; 5] = [Accent::Orange, Accent::Blue, Accent::Green, Accent::Violet, Accent::Rose];

    pub fn name(self) -> &'static str {
        match self {
            Accent::Orange => "orange",
            Accent::Blue => "blue",
            Accent::Green => "green",
            Accent::Violet => "violet",
            Accent::Rose => "rose",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.name() == name)
    }

    pub fn next(self) -> Self {
        let at = Self::ALL.iter().position(|a| *a == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }
}

/// ground, surface 1–3, track, meter, ink, dim, faint, signal, on-signal,
/// ice, on-ice, amber, on-amber, amber container, amber ink.
const fn palette(tone: Tone, c: [u32; 17]) -> Tokens {
    Tokens {
        tone,
        ground: Color::hex(c[0]),
        surface1: Color::hex(c[1]),
        surface2: Color::hex(c[2]),
        surface3: Color::hex(c[3]),
        track: Color::hex(c[4]),
        meter: Color::hex(c[5]),
        ink: Color::hex(c[6]),
        dim: Color::hex(c[7]),
        faint: Color::hex(c[8]),
        signal: Color::hex(c[9]),
        on_signal: Color::hex(c[10]),
        ice: Color::hex(c[11]),
        on_ice: Color::hex(c[12]),
        amber: Color::hex(c[13]),
        on_amber: Color::hex(c[14]),
        amber_container: Color::hex(c[15]),
        amber_ink: Color::hex(c[16]),
    }
}

impl Tokens {
    /// The palette for an accent in a tone.
    pub const fn of(accent: Accent, tone: Tone) -> Tokens {
        use Tone::{Dark, Light};
        match (accent, tone) {
            (Accent::Orange, Dark) => Tokens::dark(),
            (Accent::Orange, Light) => Tokens::light(),
            // Monet from #3d7bff
            (Accent::Blue, Dark) => palette(Dark, [0x0d0e13, 0x1a1b21, 0x202127, 0x2d2e33, 0x202127, 0x24262b, 0xeeedf4, 0xaaabb4, 0x61636c, 0x6592ff, 0x111728, 0xfdd4c0, 0x3b1b07, 0xffbf63, 0x341f00, 0x48300a, 0xffbf63]),
            (Accent::Blue, Light) => palette(Light, [0xe8e7ef, 0xfefbff, 0xf1f0f7, 0xe3e2e9, 0xfefbff, 0xe5e4ec, 0x1a1b21, 0x5c5e67, 0xaaabb4, 0x4680ff, 0xffffff, 0xfdd4c0, 0x3b1b07, 0xe6da55, 0x282400, 0xece4aa, 0x746c00]),
            // Monet from #2fb36a
            (Accent::Green, Dark) => palette(Dark, [0x0a0f0b, 0x181d18, 0x1e231e, 0x2a302b, 0x1e231e, 0x222722, 0xebefe8, 0xa5ada4, 0x5d655d, 0x23ab63, 0x091b0f, 0xfdd3d5, 0x3c181d, 0xdccd4b, 0x282400, 0x3a350b, 0xdccd4b]),
            (Accent::Green, Light) => palette(Light, [0xe5eae2, 0xf9fef6, 0xedf2ea, 0xdfe4dc, 0xf9fef6, 0xe2e7df, 0x181d18, 0x586059, 0xa5ada4, 0x009a55, 0xffffff, 0xfdd3d5, 0x3c181d, 0xe6da55, 0x282400, 0xece4aa, 0x746c00]),
            // Monet from #7a5cff
            (Accent::Violet, Dark) => palette(Dark, [0x0f0d13, 0x1c1b20, 0x222127, 0x2f2d33, 0x222127, 0x26252b, 0xf1ecf4, 0xaea9b4, 0x65626b, 0x9884ff, 0x181527, 0xfbd5bc, 0x391c04, 0xffbf63, 0x341f00, 0x48300a, 0xffbf63]),
            (Accent::Violet, Light) => palette(Light, [0xebe6ee, 0xfffbff, 0xf4eff7, 0xe6e1e9, 0xfffbff, 0xe8e4ec, 0x1c1b20, 0x605d66, 0xaea9b4, 0x886fff, 0xffffff, 0xfbd5bc, 0x391c04, 0xffcf8b, 0x332000, 0xffddb1, 0x916100]),
            // Monet from #e94560, FrcCatalyst's own red
            (Accent::Rose, Dark) => palette(Dark, [0x140c0d, 0x22191a, 0x291f20, 0x362c2c, 0x291f20, 0x2d2324, 0xfceaea, 0xbba6a7, 0x705f60, 0xff5d74, 0x271113, 0xcbe3d1, 0x032a18, 0xffbf63, 0x341f00, 0x48300a, 0xffbf63]),
            (Accent::Rose, Light) => palette(Light, [0xf6e4e4, 0xfffbff, 0xffeded, 0xf0dede, 0xfffbff, 0xf3e1e1, 0x22191a, 0x6b5a5b, 0xbba6a7, 0xee4964, 0xffffff, 0xcbe3d1, 0x032a18, 0xffcf8b, 0x332000, 0xffddb1, 0x916100]),
        }
    }
}
