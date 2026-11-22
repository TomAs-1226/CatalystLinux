//! What the offline answers cost in memory, measured: the tests' allocator
//! counts each thread's heap, now and at its highest, so a test can say what
//! reading a log or answering a question took at its peak, and what is left
//! once it is done (nothing should be).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

/// The system's allocator, counting.
pub struct Metered;

thread_local! {
    static NOW: Cell<i64> = const { Cell::new(0) };
    static PEAK: Cell<i64> = const { Cell::new(0) };
}

fn count(by: i64) {
    // Never allocates: a thread's counters are plain cells, set up as it starts.
    let _ = NOW.try_with(|now| {
        let v = now.get() + by;
        now.set(v);
        let _ = PEAK.try_with(|peak| {
            if v > peak.get() {
                peak.set(v);
            }
        });
    });
}

// SAFETY: every call is passed straight to the system's allocator; only the
// counting is added.
unsafe impl GlobalAlloc for Metered {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            count(layout.size() as i64);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            count(layout.size() as i64);
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) };
        count(-(layout.size() as i64));
    }

    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, layout, size) };
        if !q.is_null() {
            count(size as i64 - layout.size() as i64);
        }
        q
    }
}

/// This thread's heap now, the peak measured from here.
pub fn start() -> i64 {
    let now = NOW.with(Cell::get);
    PEAK.with(|p| p.set(now));
    now
}

/// The most this thread's heap rose above `base` since `start`, and where it is now.
pub fn since(base: i64) -> (i64, i64) {
    (PEAK.with(Cell::get) - base, NOW.with(Cell::get) - base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offline::facts::{analyse, MatchFacts};
    use crate::offline::synth::{write, Dip, Spec};
    use crate::offline::tools::Library;
    use crate::offline::{ask, Context};

    const MB: f64 = 1024.0 * 1024.0;

    #[test]
    fn the_analysis_path_is_light_and_lets_go_of_everything() {
        // A robot's match as big as a real one: 300 values more than the
        // faults are in, at 20 Hz for nearly three minutes.
        let mut spec = Spec::quiet("FRC_20261116_101500_NYRO_Q14.wpilog", 14);
        spec.extra = 300;
        spec.dips.push(Dip { at: 78.0, dur: 0.4, to: 6.3, load: vec![("FrontLeftDrive", 71.0)] });
        let bytes = write(&spec);
        // Reading it, as from the card: a stream, nothing of it held.
        let base = start();
        let sheet = analyse(&bytes[..], &spec.name, bytes.len() as u64).unwrap();
        let (read_peak, _) = since(base);
        let kept = sheet.to_json().to_string();
        drop(sheet);
        let (_, after_read) = since(base);
        // A question: an event's sixteen sheets read from their JSON, the
        // facts picked, the answer made; then all of it let go.
        let base = start();
        {
            let mut lib = Library::default();
            for _ in 0..16 {
                let v: serde_json::Value = serde_json::from_str(&kept).unwrap();
                lib.matches.push(MatchFacts::from_json(&v, bytes.len() as u64).unwrap());
            }
            let answer = ask("why did we brown out in Q14?", &Context::default(), &lib, None, &mut |_| {});
            assert!(answer.text.contains("browned out"), "{}", answer.text);
        }
        let (ask_peak, after_ask) = since(base);
        println!(
            "a {:.1} MB log read at a peak of {:.2} MB; its sheet {:.0} KB on the card; a question over 16 sheets at a peak of {:.2} MB; left after: {} and {} bytes",
            bytes.len() as f64 / MB,
            read_peak as f64 / MB,
            kept.len() as f64 / 1024.0,
            ask_peak as f64 / MB,
            after_read - kept.capacity() as i64,
            after_ask
        );
        assert!(read_peak < (6.0 * MB) as i64, "reading a log took {:.2} MB", read_peak as f64 / MB);
        assert!(ask_peak < (12.0 * MB) as i64, "a question took {:.2} MB", ask_peak as f64 / MB);
        assert_eq!(after_ask, 0, "nothing of a question is left once it is answered");
        assert!(after_read - kept.capacity() as i64 <= 0, "nothing of a log is left once read");
    }
}
