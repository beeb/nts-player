use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex, MutexGuard},
};

use crate::prelude::OrPanic;

/// A blocking byte FIFO with a fixed capacity, shared by one writer and one reader.
pub struct RingBuffer {
    inner: Mutex<Inner>,
    cond: Condvar,
    capacity: usize,
}

impl RingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                data: VecDeque::with_capacity(capacity),
                generation: 0,
                filling: true,
            }),
            cond: Condvar::new(),
            capacity,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.lock().data.len()
    }

    /// The current generation, see [`RingBuffer::read`].
    pub fn generation(&self) -> u32 {
        self.lock().generation
    }

    /// Whether the reader is waiting for the buffer to fill up.
    ///
    /// Also true before the first [`RingBuffer::wait_level`].
    pub fn is_filling(&self) -> bool {
        self.lock().filling
    }

    /// Append all of `data`, blocking until there is room.
    ///
    /// `data` must not be longer than the capacity, or this blocks forever.
    pub fn write(&self, data: &[u8]) {
        let mut inner = self.wait_until(|i| self.capacity - i.data.len() >= data.len());
        inner.data.extend(data);
        self.cond.notify_all();
    }

    /// Move up to `max` bytes to the end of `out`, blocking until at least one byte is available.
    ///
    /// Returns the generation of the bytes, which changes after each [`RingBuffer::clear`].
    pub fn read(&self, out: &mut Vec<u8>, max: usize) -> u32 {
        let mut inner = self.wait_until(|i| !i.data.is_empty());
        let n = max.min(inner.data.len());
        out.extend(inner.data.drain(..n));
        self.cond.notify_all();
        inner.generation
    }

    /// Drop all bytes and start a new generation.
    pub fn clear(&self) {
        let mut inner = self.lock();
        inner.data.clear();
        inner.generation = inner.generation.wrapping_add(1);
        self.cond.notify_all();
    }

    /// Block until the buffer holds at least `n` bytes.
    pub fn wait_level(&self, n: usize) {
        self.lock().filling = true;
        self.wait_until(|i| i.data.len() >= n).filling = false;
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().or_panic("ring buffer mutex poisoned")
    }

    fn wait_until(&self, ready: impl Fn(&Inner) -> bool) -> MutexGuard<'_, Inner> {
        self.cond
            .wait_while(self.lock(), |i| !ready(i))
            .or_panic("ring buffer mutex poisoned")
    }
}

struct Inner {
    data: VecDeque<u8>,
    /// Incremented by each [`RingBuffer::clear`].
    generation: u32,
    filling: bool,
}
