use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex, MutexGuard},
};

use crate::prelude::OrPanic;

/// A blocking byte FIFO with a fixed capacity, shared by one writer and one reader.
pub struct RingBuffer {
    data: Mutex<VecDeque<u8>>,
    cond: Condvar,
    capacity: usize,
}

impl RingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            data: Mutex::new(VecDeque::with_capacity(capacity)),
            cond: Condvar::new(),
            capacity,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.data
            .lock()
            .or_panic("ring buffer mutex poisoned")
            .len()
    }

    /// Append all of `data`, blocking until there is room.
    ///
    /// `data` must not be longer than the capacity, or this blocks forever.
    pub fn write(&self, data: &[u8]) {
        let mut buf = self.wait_until(|b| self.capacity - b.len() >= data.len());
        buf.extend(data);
        self.cond.notify_all();
    }

    /// Read up to `out.len()` bytes, blocking until at least one byte is available.
    pub fn read(&self, out: &mut [u8]) -> usize {
        let mut buf = self.wait_until(|b| !b.is_empty());
        let n = out.len().min(buf.len());
        for (o, b) in out.iter_mut().zip(buf.drain(..n)) {
            *o = b;
        }
        self.cond.notify_all();
        n
    }

    /// Block until the buffer holds at least `n` bytes.
    pub fn wait_level(&self, n: usize) {
        drop(self.wait_until(|b| b.len() >= n));
    }

    fn wait_until(&self, ready: impl Fn(&VecDeque<u8>) -> bool) -> MutexGuard<'_, VecDeque<u8>> {
        let buf = self.data.lock().or_panic("ring buffer mutex poisoned");
        self.cond
            .wait_while(buf, |b| !ready(b))
            .or_panic("ring buffer mutex poisoned")
    }
}
