//! Music: what plays, and the keys that steer it from anywhere.

use crate::platform::Media;

/// Play or pause, next, previous, over any screen: what the island says.
pub fn media_key(key: Media) -> Option<String> {
    let _ = key;
    Some("nothing is playing".into())
}
