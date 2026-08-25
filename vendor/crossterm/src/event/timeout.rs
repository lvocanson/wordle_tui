use std::time::Duration;
#[cfg(not(windows))]
use std::time::Instant;

// LOCAL PATCH — see …LOCAL_PATCH.md: on Windows the clock is millisecond ticks (GetTickCount64)
// instead of `Instant`. `Instant` there is QueryPerformanceCounter behind a `Once`-cached
// frequency plus 128-bit Duration arithmetic; poll timeouts only need the millisecond resolution
// that `WaitForMultipleObjects` consumes anyway. Unix keeps `Instant` (a thin clock_gettime).
#[cfg(windows)]
type Stamp = u64;
#[cfg(not(windows))]
type Stamp = Instant;

#[cfg(windows)]
extern "system" {
    fn GetTickCount64() -> u64;
}

fn stamp_now() -> Stamp {
    #[cfg(windows)]
    unsafe {
        GetTickCount64()
    }
    #[cfg(not(windows))]
    Instant::now()
}

fn stamp_elapsed(start: Stamp) -> Duration {
    #[cfg(windows)]
    return Duration::from_millis(stamp_now().wrapping_sub(start));
    #[cfg(not(windows))]
    start.elapsed()
}

/// Keeps track of the elapsed time since the moment the polling started.
#[derive(Debug, Clone)]
pub struct PollTimeout {
    timeout: Option<Duration>,
    start: Stamp,
}

impl PollTimeout {
    /// Constructs a new `PollTimeout` with the given optional `Duration`.
    pub fn new(timeout: Option<Duration>) -> PollTimeout {
        PollTimeout {
            timeout,
            start: stamp_now(),
        }
    }

    /// Returns whether the timeout has elapsed.
    ///
    /// It always returns `false` if the initial timeout was set to `None`.
    pub fn elapsed(&self) -> bool {
        self.timeout
            .map(|timeout| stamp_elapsed(self.start) >= timeout)
            .unwrap_or(false)
    }

    /// Returns the timeout leftover (initial timeout duration - elapsed duration).
    pub fn leftover(&self) -> Option<Duration> {
        self.timeout.map(|timeout| {
            let elapsed = stamp_elapsed(self.start);

            if elapsed >= timeout {
                Duration::from_secs(0)
            } else {
                timeout - elapsed
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::PollTimeout;

    #[test]
    pub fn test_timeout_without_duration_does_not_have_leftover() {
        let timeout = PollTimeout::new(None);
        assert_eq!(timeout.leftover(), None)
    }

    #[test]
    pub fn test_timeout_without_duration_never_elapses() {
        let timeout = PollTimeout::new(None);
        assert!(!timeout.elapsed());
    }

    #[test]
    pub fn test_timeout_elapses() {
        const TIMEOUT_MILLIS: u64 = 100;

        let timeout = PollTimeout {
            timeout: Some(Duration::from_millis(TIMEOUT_MILLIS)),
            start: Instant::now() - Duration::from_millis(2 * TIMEOUT_MILLIS),
        };

        assert!(timeout.elapsed());
    }

    #[test]
    pub fn test_elapsed_timeout_has_zero_leftover() {
        const TIMEOUT_MILLIS: u64 = 100;

        let timeout = PollTimeout {
            timeout: Some(Duration::from_millis(TIMEOUT_MILLIS)),
            start: Instant::now() - Duration::from_millis(2 * TIMEOUT_MILLIS),
        };

        assert!(timeout.elapsed());
        assert_eq!(timeout.leftover(), Some(Duration::from_millis(0)));
    }

    #[test]
    pub fn test_not_elapsed_timeout_has_positive_leftover() {
        let timeout = PollTimeout::new(Some(Duration::from_secs(60)));

        assert!(!timeout.elapsed());
        assert!(timeout.leftover().unwrap() > Duration::from_secs(0));
    }
}
