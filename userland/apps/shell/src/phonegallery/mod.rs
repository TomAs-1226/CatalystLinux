//! The phone gallery (1.5.0): the album on a phone, in its browser, over
//! the Wi-Fi they share or the phone's own hotspot. No cable, no app.
//!
//! Photos › `p` (or Share › Photos) shows a QR code and the address with a
//! six-digit code. The phone scans the code (or types the address and the
//! digits) and gets one page: the album newest first, by day, each photo's
//! caption, place and match (photolog/), full screen with a swipe, saved
//! one at a time or all together as a ZIP.
//!
//! **Private by default.** Nothing listens except while the sheet shows on
//! the handheld, and then for ten minutes after it was last used (a phone
//! loading photos, or `+`); Esc stops it at once, closing what is under way.
//! The code's link carries a token of 130 random bits, new each time; it
//! (or the six digits) is traded for a cookie, and nothing is served
//! without one. Five wrong codes and sharing stops. Only names the album
//! lists are served, from its folder, read-only: nothing on the page
//! deletes or uploads. A request naming the handheld by anything but an
//! address is refused (no DNS rebinding), and eight connections at once at
//! most. The token, the code and the cookies are never written to a log.
//!
//! It is plain HTTP on the local network: a browser keeps the Web Share
//! API for secure pages, so "share" is a download the phone then shares
//! (or a long press on the photo); where `navigator.share` with files is
//! there after all, the page uses it.

mod page;
mod serve;
mod sheet;
pub mod zip;
#[cfg(not(target_arch = "aarch64"))]
pub mod shots;
#[cfg(test)]
mod tests;

use std::path::Path;

use crate::photolog::meta::About;
use crate::photos::Photo;

pub use page::Look;
pub use serve::{Gallery, Stopped, Trouble};
pub use sheet::GalleryScreen;

/// A photo as the gallery offers it: the album's, with what its sidecar
/// says, and what it is to a browser.
#[derive(Clone, Debug)]
pub struct Item {
    pub photo: Photo,
    pub about: About,
    pub mime: &'static str,
}

impl Item {
    /// Played rather than looked at: a loop (GIF) or a video.
    pub fn moving(&self) -> bool {
        self.mime == "image/gif" || self.video()
    }

    pub fn video(&self) -> bool {
        self.mime.starts_with("video/")
    }

    /// A video a phone's browser plays in the page. A time-lapse (Motion
    /// JPEG in an AVI, lapse.rs) plays only once saved: the viewer shows its
    /// first frame and the download.
    pub fn inline(&self) -> bool {
        matches!(self.mime, "video/mp4" | "video/webm" | "video/quicktime")
    }

    /// A pixel picture (the Camera's pixel mode): shown with whole pixels.
    pub fn pixel(&self) -> bool {
        crate::photos::is_pixel(&self.photo.name)
    }
}

/// What a file is to a browser, by its extension.
pub fn mime_of(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "avi" => "video/x-msvideo",
        _ => "application/octet-stream",
    }
}

/// The album in `album`, as its own listing has it (photos.rs), each with
/// its sidecar read.
pub fn items(album: &Path) -> Vec<Item> {
    crate::photos::list_in(album)
        .into_iter()
        .map(|photo| Item { about: crate::photolog::meta::read(&photo.path), mime: mime_of(&photo.name), photo })
        .collect()
}
