//! A stopwatch for the stage timings the statistics report. In a browser
//! (`wasm32-unknown-unknown`) `std::time::Instant::now` panics, so there it
//! measures nothing and every span reads zero; no output depends on it.

use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub struct Stopwatch {
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    start: std::time::Instant,
}

impl Stopwatch {
    pub fn start() -> Self {
        Self {
            #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
            start: std::time::Instant::now(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            self.start.elapsed()
        }
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            Duration::ZERO
        }
    }
}
