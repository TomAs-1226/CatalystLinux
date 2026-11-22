//! Copying from the vault: the shell's own clipboard (`clipboard/`), every
//! copy a secret. A secret is never written to the card and never kept in
//! the history; it clears after 30 seconds, and at once when the handheld
//! locks. What the vault copies can be pasted anywhere until then.

pub use crate::clipboard::{Sensitivity, SECRET_FOR};

/// Put `text` on the clipboard as `sensitivity` says (the vault: always a
/// secret). The island says it was copied and that it clears in 30 s.
pub fn copy(text: &str, sensitivity: Sensitivity) {
    crate::clipboard::copy(text, sensitivity);
    #[cfg(test)]
    tests::record(text, sensitivity);
}

/// Asked now and then by the vault: a secret whose 30 seconds are up goes.
/// (The shell's own look does the same, every frame.)
pub fn clear_after() {
    crate::clipboard::tick();
}

/// The clipboard's history, as text: secrets are never in it.
#[cfg(test)]
pub fn history() -> Vec<String> {
    crate::clipboard::with(|b| b.items().into_iter().map(|i| i.text).collect())
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
    fn a_secret_copied_from_the_vault_pastes_and_stays_out_of_the_history() {
        copy("hunter2-demo", Sensitivity::Secret);
        assert_eq!(crate::clipboard::paste_text().as_deref(), Some("hunter2-demo"), "it pastes");
        assert!(history().is_empty(), "never in the history");
        // The handheld locking takes it at once.
        crate::clipboard::lock();
        assert!(crate::clipboard::paste_text().is_none());
    }
}
