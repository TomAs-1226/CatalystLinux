//! Every photo's finish (1.5.2): the words that ask catalyst-camera for a
//! photo merged from a few frames taken back to back, its local tone and its
//! own sharpening (stack.h's `merge_frames`, `finish`), with the ISP's
//! sharpening turned down to make room for it.
//!
//! - **Frames.** Photo and Pro mode take three frames a seventh of a second
//!   apart (about 0.3 s more for the hand than one), held by the camera
//!   program and merged as the photo is written, in the background behind
//!   the preview: the shutter is as quick as ever, the photo about a second
//!   longer to save. Detail mode (the camera bar's chip) now says how many:
//!   auto three, on six, off one. Its old background burst (burst.rs) is no
//!   longer asked for: the merge in the camera program does its work, better
//!   aligned and without a second copy of every frame in memory.
//! - **Never merged:** a RAW (the memory is the RAW's), a bracketing set,
//!   Pro's exposures longer than a fifteenth of a second (three of those
//!   would keep the hand still too long), Night and HDR (their own stacks),
//!   burst, trails, time-lapse, pixel and document modes.
//! - **Local tone** in photo mode, and Night's and HDR's: in full with no
//!   look, gently under a look (whose own curve decides the style, as 1.5.1
//!   keeps the toe off then); none in Pro (the dials decide the exposure).
//! - **Sharpening**: the camera program's own, after the merge, on the luma
//!   (edges and texture, not flat sky or skin; less as the noise rises),
//!   with the ISP's down to a quarter so the frames are merged before they
//!   are sharpened.

use crate::burst::DetailMode;

/// The frames a photo merges, by Detail mode.
pub fn frames(detail: DetailMode) -> u32 {
    match detail {
        DetailMode::Auto => 3,
        DetailMode::On => 6,
        DetailMode::Off => 1,
    }
}

/// Pro's longest exposure that still merges: a fifteenth of a second (the
/// longest the camera's own exposure takes in a photo).
pub const MERGE_LONGEST_US: u32 = 66_667;

/// The ISP's sharpening under the finish: a quarter of its own (1.5.1 had it
/// in full), so the frames are merged before they are sharpened and the
/// finish's sharpening, which knows the noise left, does the rest.
pub const ISP_SHARPNESS: f32 = 0.25;

/// The finish's sharpening (0 to 1; the camera program lowers it as the
/// noise rises).
pub const CRISP: f32 = 0.6;

/// The local tone's strength: in full, or gently under a look.
pub fn local(look_on: bool) -> f32 {
    if look_on {
        0.6
    } else {
        1.0
    }
}

/// The finish's words laid into a still order's: the ISP's sharpening
/// turned down (any `sharp=` replaced), `merge=` where more than one frame,
/// `local=` where asked and `crisp=`.
pub fn add_words(words: &mut Vec<String>, frames: u32, local: f32) {
    words.retain(|w| !w.starts_with("sharp=") && !w.starts_with("merge=") && !w.starts_with("local=") && !w.starts_with("crisp="));
    words.push(format!("sharp={ISP_SHARPNESS:.2}"));
    if frames > 1 {
        words.push(format!("merge={frames}"));
    }
    if local > 0.0 {
        words.push(format!("local={local:.2}"));
    }
    words.push(format!("crisp={CRISP:.2}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_finish_words_replace_the_isp_sharpening_and_ask_for_the_merge() {
        let mut words = vec!["exp=10000".to_owned(), "sharp=1.00".to_owned(), "nr=fast".to_owned()];
        add_words(&mut words, frames(DetailMode::Auto), local(false));
        assert_eq!(words, ["exp=10000", "nr=fast", "sharp=0.25", "merge=3", "local=1.00", "crisp=0.60"]);
        // Under a look: the local tone gentle; Detail off: one frame; Pro: no local tone.
        let mut words = vec!["sharp=1.00".to_owned()];
        add_words(&mut words, frames(DetailMode::Off), 0.0);
        assert_eq!(words, ["sharp=0.25", "crisp=0.60"]);
        assert_eq!(local(true), 0.6);
        assert_eq!(frames(DetailMode::On), 6);
    }
}
