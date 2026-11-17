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
