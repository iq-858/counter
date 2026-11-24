//! A short, precise wait for the last slice before a frame deadline.
//!
//! `thread::sleep` follows the platform's coarse timer. On Windows that timer
//! can round a 1 ms request up to a 15.6 ms tick, long enough to miss a 60 fps
//! deadline. The coarse part of a wait still uses `recv_timeout` so commands
//! reach the decode thread; this helper handles only the final slice. Windows
//! uses a high-resolution waitable timer when it can, which sleeps without
//! burning a core. Other platforms sleep the bulk of the slice and spin only
//! its last fraction of a millisecond.

use std::time::{Duration, Instant};

/// The tail of a short wait spent spinning.
///
/// A sleep's own granularity is coarser than a frame deadline, so the last
/// fraction of a millisecond is spun instead. Spinning the *whole* slice — what
/// this used to do — costs a full core for up to 16 ms on every displayed
/// picture: about 96% of one core at 60 fps and 48% at 30 fps on Android and
/// iOS, where this fallback is the only path. That is battery drain and thermal
/// throttling in the one place a video player must sleep.
const SPIN_TAIL: Duration = Duration::from_micros(300);

#[cfg(target_os = "windows")]
mod imp {
    use std::time::Duration;

    use windows::Win32::{
        Foundation::HANDLE,
        System::Threading::{
            CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, INFINITE,
            SetWaitableTimer, TIMER_ALL_ACCESS, WaitForSingleObject,
        },
    };

    pub(crate) struct FineWait {
        timer: Option<HANDLE>,
    }

    impl FineWait {
        pub(crate) fn new() -> Self {
            let timer = unsafe {
                CreateWaitableTimerExW(
                    None,
                    windows::core::PCWSTR::null(),
                    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                    TIMER_ALL_ACCESS.0,
                )
                .ok()
            };
            Self { timer }
        }

        pub(crate) fn wait(&self, duration: Duration) {
            if let Some(timer) = self.timer {
                // Waitable timers take relative time as negative 100 ns units.
                let units = duration.as_nanos().div_ceil(100).min(i64::MAX as u128) as i64;
                let due = -units.max(1);
                let armed = unsafe { SetWaitableTimer(timer, &due, 0, None, None, false).is_ok() };
                if armed {
                    unsafe {
                        let _ = WaitForSingleObject(timer, INFINITE);
                    }
                    return;
                }
            }
            // A system without the high-resolution timer sleeps most of the
            // wait and spins only its last fraction of a millisecond, exactly
            // as the other platforms do. It never spins the whole slice.
            super::yield_until(duration);
        }
    }

    impl Drop for FineWait {
        fn drop(&mut self) {
            if let Some(timer) = self.timer.take() {
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(timer);
                }
            }
        }
    }

    // The handle is owned by this struct and used only from the decode thread
    // that owns the struct, but the Windows wrapper is a raw pointer and is not
    // `Send` on its own.
    unsafe impl Send for FineWait {}
}

#[cfg(not(target_os = "windows"))]
mod imp {
    use std::time::Duration;

    use super::yield_until;

    pub(crate) struct FineWait;

    impl FineWait {
        pub(crate) fn new() -> Self {
            Self
        }

        pub(crate) fn wait(&self, duration: Duration) {
            yield_until(duration);
        }
    }
}

/// A fallback for platforms without a precise sleeping primitive.
///
/// It sleeps the bulk of the wait so the core stays free for the decoder and
/// the audio callback, then spins only [`SPIN_TAIL`] to land on the deadline.
fn yield_until(duration: Duration) -> Duration {
    let deadline = Instant::now() + duration;
    if let Some(bulk) = duration.checked_sub(SPIN_TAIL) {
        std::thread::sleep(bulk);
    }
    let spin_start = Instant::now();
    let mut spins = 0u32;
    while Instant::now() < deadline {
        std::hint::spin_loop();
        spins = spins.wrapping_add(1);
        // Hand the core back periodically so a single-core phone can run its
        // audio callback during the tail instead of waiting for a scheduler
        // tick. The yield costs far less than the sleep it replaced.
        if spins % 64 == 0 {
            std::thread::yield_now();
        }
    }
    spin_start.elapsed()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A regression to the old full-slice spin would spend the whole wait in
    /// the loop instead of the tail. The bound is generous so a preempted test
    /// thread does not fail it, yet far below the duration it must not burn.
    #[test]
    fn the_tail_is_spun_but_the_bulk_is_slept() {
        let duration = Duration::from_millis(20);
        let start = Instant::now();
        let spun = yield_until(duration);
        let elapsed = start.elapsed();
        assert!(
            elapsed >= duration,
            "the wait returned after {elapsed:?}, before its {duration:?} deadline"
        );
        assert!(
            spun <= SPIN_TAIL + Duration::from_millis(10),
            "spun {spun:?} of a {duration:?} wait; only the {SPIN_TAIL:?} tail may spin"
        );
    }
}

pub(crate) use imp::FineWait;
