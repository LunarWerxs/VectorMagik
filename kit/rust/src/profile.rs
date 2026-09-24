//! Wall-clock accounting per named span, compiled in only with the `profile`
//! feature (`cargo build --features profile`); without it every span is a
//! no-op the optimiser removes. Spans nest freely; each name accumulates the
//! time and the number of entries on the current thread.
#[cfg(feature = "profile")]
mod enabled {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    thread_local! {
        static SPANS: RefCell<BTreeMap<&'static str, (Duration, u64)>> =
            const { RefCell::new(BTreeMap::new()) };
    }

    /// An open span; dropping it books the elapsed time under its name.
    pub struct Span {
        name: &'static str,
        start: Instant,
    }

    pub fn span(name: &'static str) -> Span {
        Span {
            name,
            start: Instant::now(),
        }
    }

    impl Drop for Span {
        fn drop(&mut self) {
            let elapsed = self.start.elapsed();
            SPANS.with(|spans| {
                let mut spans = spans.borrow_mut();
                let entry = spans.entry(self.name).or_insert((Duration::ZERO, 0));
                entry.0 += elapsed;
                entry.1 += 1;
            });
        }
    }

    /// One line per span, slowest first: name, seconds, entries.
    pub fn report() -> String {
        SPANS.with(|spans| {
            let spans = spans.borrow();
            let mut rows: Vec<_> = spans.iter().collect();
            rows.sort_by_key(|row| std::cmp::Reverse(row.1 .0));
            rows.iter()
                .map(|(name, (time, count))| {
                    format!("{name}: {:.3}s over {count} entries", time.as_secs_f64())
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    pub fn reset() {
        SPANS.with(|spans| spans.borrow_mut().clear());
    }
}

#[cfg(feature = "profile")]
pub use enabled::{report, reset, span, Span};

/// Without the feature a span is an empty value that costs nothing to bind.
#[cfg(not(feature = "profile"))]
pub struct Span;

#[cfg(not(feature = "profile"))]
#[inline(always)]
pub fn span(_name: &'static str) -> Span {
    Span
}

#[cfg(not(feature = "profile"))]
pub fn report() -> String {
    String::new()
}

#[cfg(not(feature = "profile"))]
pub fn reset() {}
