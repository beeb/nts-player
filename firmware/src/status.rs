use std::sync::Mutex;

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
pub struct Status(Mutex<State>);

impl Status {
    pub fn get(&self) -> State {
        *self.0.lock().or_panic("status mutex poisoned")
    }

    pub fn set(&self, state: State) {
        *self.0.lock().or_panic("status mutex poisoned") = state;
    }
}
