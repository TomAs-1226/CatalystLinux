//! Copying from the vault, under the name the system clipboard will have.
//!
//! The shell's clipboard, with its history, is being built beside this
//! (another branch). Its call will be `crate::clipboard::copy(text,
//! Sensitivity::Secret)`: a secret is never stored on the card, never kept
//! in the history, and cleared after 30 seconds. Until it lands, this shim
//! is that call: it holds the one secret copied, in memory that is wiped,
//! keeps no history at all, and clears it after 30 seconds, whether the
//! vault is still open or not (as the real one will).
//!
//! **When the clipboard lands**, replace everything above the tests here
//! with:
//!
//! ```ignore
//! pub use crate::clipboard::{copy, Sensitivity};
//! ```
//!
//! and point [`held`] and [`clear_after`]'s test at the clipboard's own
//! (its history, its current text). The vault calls nothing else.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

/// How a copied text is to be treated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sensitivity {
    /// A password, a PIN, a code: never stored, never in the history,
    /// cleared after [`SECRET_FOR`].
    Secret,
}

/// How long a secret stays on the clipboard.
pub const SECRET_FOR: Duration = Duration::from_secs(30);

struct Held {
    text: Zeroizing<String>,
    at: Instant,
}

#[cfg(not(test))]
fn held_now() -> std::sync::MutexGuard<'static, Option<Held>> {
    static HELD: Mutex<Option<Held>> = Mutex::new(None);
    HELD.lock().unwrap_or_else(|e| e.into_inner())
}

// Each test thread copies to a clipboard of its own, so one test's copy is
// never another's.
#[cfg(test)]
fn held_now() -> std::sync::MutexGuard<'static, Option<Held>> {
    thread_local! {
        static CELL: &'static Mutex<Option<Held>> = Box::leak(Box::new(Mutex::new(None)));
    }
    CELL.with(|c| *c).lock().unwrap_or_else(|e| e.into_inner())
}

/// Put `text` on the clipboard as `sensitivity` says.
pub fn copy(text: &str, sensitivity: Sensitivity) {
    match sensitivity {
        Sensitivity::Secret => {
            *held_now() = Some(Held { text: Zeroizing::new(text.to_owned()), at: Instant::now() });
        }
    }
    #[cfg(test)]
    tests::record(text, sensitivity);
}

/// What the clipboard holds now (a secret within its 30 seconds).
#[allow(dead_code)]
pub fn held() -> Option<Zeroizing<String>> {
    clear_after();
    held_now().as_ref().map(|h| h.text.clone())
}

/// Clear a secret whose 30 seconds are up (asked now and then by the vault).
pub fn clear_after() {
    let mut held = held_now();
    if held.as_ref().is_some_and(|h| h.at.elapsed() >= SECRET_FOR) {
        *held = None;
    }
}

/// The clipboard's history: secrets are never in it, and this shim keeps
/// nothing else, so it is always empty.
#[allow(dead_code)]
pub fn history() -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    thread_local! {
        static COPIED: std::cell::RefCell<Vec<(String, Sensitivity)>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    pub fn record(text: &str, sensitivity: Sensitivity) {
        COPIED.with(|c| c.borrow_mut().push((text.to_owned(), sensitivity)));
    }

    /// Everything this thread copied, and how.
    pub fn copied() -> Vec<(String, Sensitivity)> {
        COPIED.with(|c| c.borrow().clone())
    }

    #[test]
    fn a_secret_is_held_without_history_and_cleared_after_thirty_seconds() {
        copy("hunter2-demo", Sensitivity::Secret);
        assert_eq!(held().as_deref().map(String::as_str), Some("hunter2-demo"));
        assert!(history().is_empty());
        // Thirty seconds on.
        held_now().as_mut().unwrap().at -= SECRET_FOR;
        clear_after();
        assert!(held().is_none());
    }
}
