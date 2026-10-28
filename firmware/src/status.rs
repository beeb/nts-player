use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

use crate::prelude::OrPanic;

/// Device state shown on the LED ring.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    /// Connecting or reconnecting to Wi-Fi.
    #[default]
    Connecting,

    /// The setup portal is running.
    Portal,

    /// Connected to wifi.
    Online,

    /// A fatal error happened, the device restarts soon.
    Fatal,
}

#[derive(Default)]
pub struct Status {
    state: Mutex<State>,
    /// Whether the stream failed since it last received audio.
    stalled: AtomicBool,
}

impl Status {
    pub fn get(&self) -> State {
        *self.state.lock().or_panic("status mutex poisoned")
    }

    pub fn set(&self, state: State) {
        *self.state.lock().or_panic("status mutex poisoned") = state;
    }

    pub fn is_stalled(&self) -> bool {
        self.stalled.load(Ordering::Relaxed)
    }

    pub fn set_stalled(&self, stalled: bool) {
        self.stalled.store(stalled, Ordering::Relaxed);
    }
}
