//! Bezel on the CPU.
//!
//! The Bezel design language (Material 3 tonal surfaces, Liquid Glass only on
//! floating controls, Detent motion) drawn into a plain RGB framebuffer for
//! small panels with no GPU in the path. A frame is three steps, as in the
//! Bezel specimen:
//!
//! 1. the scene: ground, tonal surfaces, text and icons ([`Canvas`], [`Text`], [`Icons`]);
//! 2. the glass pass over that scene ([`Glass`]);
//! 3. whatever sits on the glass (icons and labels), drawn on top.

pub mod canvas;
pub mod color;
pub mod glass;
pub mod icons;
pub mod text;

pub use canvas::{Canvas, Rect};
pub use color::{Color, Tokens, Tone};
pub use glass::{Glass, GlassShape, LIGHT_REST};
pub use icons::Icons;
pub use text::{Align, Face, Text, TextStyle};
