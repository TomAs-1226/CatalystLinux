//! # Catalyst Kit
//!
//! Write an app for Catalyst Linux that looks and moves like the shell's
//! own: the same surfaces, type and glass (Bezel), the same springs
//! (Detent), the same keys, in the look the handheld is wearing. The shell
//! hands the app the panel when its tile is opened and takes it back when
//! the app ends, or when Ctrl-Q is pressed or Esc held.
//!
//! ```no_run
//! use catalyst_kit::prelude::*;
//!
//! struct Hello { list: List }
//!
//! impl Page for Hello {
//!     fn title(&self) -> &str { "Hello" }
//!     fn key(&mut self, key: Key, cx: &mut Cx) -> Flow {
//!         match key {
//!             Key::Back => Flow::Quit,
//!             Key::Enter => { cx.say("hello"); Flow::Stay }
//!             other => { self.list.key(other, 3); Flow::Stay }
//!         }
//!     }
//!     fn draw(&mut self, ui: &mut Ui) {
//!         let (top, h) = (ui.top(), ui.row_h());
//!         let bottom = ui.bottom();
//!         for (i, y) in self.list.layout(ui, 3, top, h, 3.0, bottom) {
//!             let ink = ui.tokens.dim;
//!             ui.row(Rect::new(PAD, y, W - 2.0 * PAD, h), ["one", "two", "three"][i], "", ink, i == self.list.focus);
//!         }
//!         self.list.end(ui, 3, top, bottom);
//!         ui.hint("enter: say hello · esc: leave");
//!     }
//! }
//!
//! fn main() { catalyst_kit::launch(|| Hello { list: List::default() }, &[]) }
//! ```
//!
//! What is here:
//!
//! - [`Page`], [`Flow`] and [`Cx`]: an app's pages and what a key leads to.
//! - [`Ui`]: drawing, measured from the type ([`Ui::top`], [`Ui::row_h`]),
//!   with the controls every page shares ([`Ui::row`], [`Ui::switch_row`],
//!   [`Ui::button`], [`Ui::chip`], [`Ui::meter`], [`Ui::stat`], [`Ui::hint`])
//!   and the focus ring ([`Ui::focus_ring`]); [`List`] for lists.
//! - [`Key`]: the handheld's keys, read as the shell reads them.
//! - [`Look`]: the look the handheld wears, from the shell.
//! - [`store`]: the app's own folder, kept across updates, and [`Saved`]
//!   values; [`Rng`] for games.
//! - [`detent`] and [`bezel`], re-exported, for springs and the renderer.

pub mod app;
pub mod field;
pub mod keys;
pub mod look;
pub mod panel;
pub mod store;
pub mod sys;
pub mod ui;

pub use app::{launch, run, Cx, Flow, Page, Runner, Scene};
pub use bezel;
pub use bezel::{Accent, Align, Color, Rect, TextStyle, Tokens, Tone};
pub use detent;
pub use detent::{role, Animated, Spring};
pub use field::Field;
pub use keys::Key;
pub use look::{Look, TextSize, Theme, Type};
pub use store::{data_dir, Rng, Saved};
pub use ui::{spatial, List, Ui, GAP, H, PAD, TILE_RADIUS, W};

/// Everything an app usually needs, in one `use`.
pub mod prelude {
    pub use crate::{launch, role, spatial, Align, Animated, Color, Cx, Field, Flow, Key, List, Look, Page, Rect, Rng, Saved, TextStyle, Ui, GAP, H, PAD, TILE_RADIUS, W};
}
