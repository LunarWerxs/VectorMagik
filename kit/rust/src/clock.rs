//! A stopwatch for the stage timings the statistics report and the app's
//! clocks. In a browser (`wasm32-unknown-unknown`) `std::time::Instant::now`
//! panics, so there it reads the page's clock (`performance.now()`), which
//! the module's loader supplies as `env.vm_now_ms`. No output depends on it.

use std::time::Duration;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[link(wasm_import_module = "env")]
extern "C" {
    /// Milliseconds on the page's monotonic clock.
    fn vm_now_ms() -> f64;
}

#[derive(Clone, Copy, Debug)]
pub struct Stopwatch {
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    start: std::time::Instant,
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    start_ms: f64,
}

impl Stopwatch {
    pub fn start() -> Self {
        Self {
            #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
            start: std::time::Instant::now(),
            // SAFETY: the loader supplies the import; it takes and keeps nothing.
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            start_ms: unsafe { vm_now_ms() },
        }
    }

    pub fn elapsed(&self) -> Duration {
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            self.start.elapsed()
        }
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            // SAFETY: as in `start`.
            let now = unsafe { vm_now_ms() };
            Duration::from_secs_f64(((now - self.start_ms) / 1000.).max(0.))
        }
    }
}
