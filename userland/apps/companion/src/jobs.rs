//! Work done away from the window (the network, the sign-in window,
//! files), and how far it has got.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// One piece of work at a time, its result kept until taken.
pub struct Job<T> {
    slot: Arc<Mutex<Option<T>>>,
    busy: Arc<AtomicBool>,
}

impl<T: Send + 'static> Job<T> {
    pub fn new() -> Self {
        Job { slot: Arc::new(Mutex::new(None)), busy: Arc::new(AtomicBool::new(false)) }
    }

    /// Started, unless it is under way already.
    pub fn start(&self, work: impl FnOnce() -> T + Send + 'static) {
        if self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let (slot, busy) = (self.slot.clone(), self.busy.clone());
        std::thread::spawn(move || {
            let done = work();
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(done);
            busy.store(false, Ordering::SeqCst);
        });
    }

    pub fn take(&self) -> Option<T> {
        self.slot.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    pub fn busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }
}

/// How far something has got, 0 to 1, shared with the thread doing it.
#[derive(Clone, Debug, Default)]
pub struct Progress(Arc<AtomicU32>);

impl Progress {
    pub fn set(&self, fraction: f32) {
        self.0.store((fraction.clamp(0.0, 1.0) * 1_000_000.0) as u32, Ordering::Relaxed);
    }

    pub fn get(&self) -> f32 {
        self.0.load(Ordering::Relaxed) as f32 / 1_000_000.0
    }
}

/// A pipe between two threads: what one writes, the other reads.
pub fn pipe() -> (PipeWriter, PipeReader) {
    let (tx, rx) = std::sync::mpsc::sync_channel(16);
    (PipeWriter(tx), PipeReader { rx, buf: Vec::new(), pos: 0 })
}

pub struct PipeWriter(std::sync::mpsc::SyncSender<Vec<u8>>);

impl std::io::Write for PipeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.send(buf.to_vec()).map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "the other end stopped"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub struct PipeReader {
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
}

impl std::io::Read for PipeReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.pos >= self.buf.len() {
            match self.rx.recv() {
                Ok(chunk) => {
                    self.buf = chunk;
                    self.pos = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}
