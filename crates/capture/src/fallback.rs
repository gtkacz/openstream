//! Tries a primary capture path and, when it fails to start, dies, or delivers no frame in time,
//! an optional fallback. Platform neutral so the decision logic is tested without a display.

use std::time::Duration;

use crate::error::CaptureError;
use crate::frame::{CaptureSession, SourceInfo};

/// A capture path that is running but has not yet delivered a frame. Dropping it must stop the
/// capture: the driver drops a silent primary before it starts the fallback.
pub trait Started {
    /// Blocks until the first frame is reported (`Ok(Some)`), `timeout` passes (`Ok(None)`), or
    /// the capture ends before producing one (`Err`).
    fn wait_first_frame(&mut self, timeout: Duration) -> Result<Option<SourceInfo>, CaptureError>;
    /// Turns the proven capture into the session the room owns.
    fn into_session(self: Box<Self>, info: SourceInfo) -> Box<dyn CaptureSession>;
}

/// One way to open a source: a name for error messages and the function that starts it.
pub struct Attempt<'a> {
    pub name: &'static str,
    pub start: Box<dyn FnOnce() -> Result<Box<dyn Started>, CaptureError> + 'a>,
}

/// Returns the first attempt that delivers a frame within `timeout`. A primary that fails to
/// start, ends early, or stays silent is dropped, and therefore stopped, before the fallback
/// starts. With nothing left to try, the error names every attempt and why it failed.
pub fn start_with_fallback(
    timeout: Duration,
    primary: Attempt<'_>,
    fallback: Option<Attempt<'_>>,
) -> Result<Box<dyn CaptureSession>, CaptureError> {
    let mut failures = Vec::new();
    for attempt in std::iter::once(primary).chain(fallback) {
        match (attempt.start)() {
            Ok(mut started) => match started.wait_first_frame(timeout) {
                Ok(Some(info)) => return Ok(started.into_session(info)),
                // `started` drops at the end of this arm, so the capture has stopped by the time
                // the loop starts the next attempt.
                Ok(None) => {
                    failures.push(format!("{}: no frame within {:?}", attempt.name, timeout))
                }
                Err(error) => failures.push(format!("{}: {error}", attempt.name)),
            },
            Err(error) => failures.push(format!("{}: {error}", attempt.name)),
        }
    }
    Err(CaptureError::SourceLost(failures.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    const INFO_A: SourceInfo = SourceInfo {
        width: 64,
        height: 32,
        fps: 30,
    };
    const INFO_B: SourceInfo = SourceInfo {
        width: 128,
        height: 64,
        fps: 60,
    };
    const TIMEOUT: Duration = Duration::from_millis(10);

    struct FakeSession(SourceInfo);

    impl CaptureSession for FakeSession {
        fn info(&self) -> SourceInfo {
            self.0
        }
        fn stop(self: Box<Self>) {}
    }

    /// Answers the first-frame wait at once with a fixed outcome; flips `stopped` when dropped.
    struct FakeStarted {
        outcome: Result<Option<SourceInfo>, &'static str>,
        stopped: Arc<AtomicBool>,
    }

    impl Started for FakeStarted {
        fn wait_first_frame(&mut self, _: Duration) -> Result<Option<SourceInfo>, CaptureError> {
            self.outcome
                .map_err(|message| CaptureError::SourceLost(message.into()))
        }
        fn into_session(self: Box<Self>, info: SourceInfo) -> Box<dyn CaptureSession> {
            Box::new(FakeSession(info))
        }
    }

    impl Drop for FakeStarted {
        fn drop(&mut self) {
            self.stopped.store(true, Ordering::SeqCst);
        }
    }

    struct Probe {
        called: Arc<AtomicBool>,
        stopped: Arc<AtomicBool>,
    }

    impl Probe {
        fn new() -> Self {
            Self {
                called: Arc::default(),
                stopped: Arc::default(),
            }
        }
        fn called(&self) -> bool {
            self.called.load(Ordering::SeqCst)
        }
        fn stopped(&self) -> bool {
            self.stopped.load(Ordering::SeqCst)
        }
        /// An attempt that starts and then answers the wait with `outcome`.
        fn starting(
            &self,
            name: &'static str,
            outcome: Result<Option<SourceInfo>, &'static str>,
        ) -> Attempt<'static> {
            let (called, stopped) = (self.called.clone(), self.stopped.clone());
            Attempt {
                name,
                start: Box::new(move || {
                    called.store(true, Ordering::SeqCst);
                    Ok(Box::new(FakeStarted { outcome, stopped }) as Box<dyn Started>)
                }),
            }
        }
        /// An attempt whose start itself fails.
        fn failing(&self, name: &'static str, message: &'static str) -> Attempt<'static> {
            let called = self.called.clone();
            Attempt {
                name,
                start: Box::new(move || {
                    called.store(true, Ordering::SeqCst);
                    Err(CaptureError::Windows(message.into()))
                }),
            }
        }
    }

    #[test]
    fn a_primary_that_delivers_is_used_and_the_fallback_never_starts() {
        let (primary, fallback) = (Probe::new(), Probe::new());
        let session = start_with_fallback(
            TIMEOUT,
            primary.starting("graphics capture", Ok(Some(INFO_A))),
            Some(fallback.starting("desktop duplication", Ok(Some(INFO_B)))),
        )
        .unwrap();
        assert_eq!(session.info(), INFO_A);
        assert!(!fallback.called());
    }

    #[test]
    fn a_silent_primary_is_stopped_before_the_fallback_starts() {
        let (primary, fallback) = (Probe::new(), Probe::new());
        let primary_stopped = primary.stopped.clone();
        let fallback_attempt = Attempt {
            name: "desktop duplication",
            start: Box::new({
                let (called, stopped) = (fallback.called.clone(), fallback.stopped.clone());
                move || {
                    called.store(true, Ordering::SeqCst);
                    assert!(
                        primary_stopped.load(Ordering::SeqCst),
                        "the fallback started while the primary still ran"
                    );
                    Ok(Box::new(FakeStarted {
                        outcome: Ok(Some(INFO_B)),
                        stopped,
                    }) as Box<dyn Started>)
                }
            }),
        };
        let session = start_with_fallback(
            TIMEOUT,
            primary.starting("graphics capture", Ok(None)),
            Some(fallback_attempt),
        )
        .unwrap();
        assert_eq!(session.info(), INFO_B);
        assert!(fallback.called());
    }

    #[test]
    fn a_primary_that_fails_to_start_hands_over_to_the_fallback() {
        let (primary, fallback) = (Probe::new(), Probe::new());
        let session = start_with_fallback(
            TIMEOUT,
            primary.failing("graphics capture", "unsupported"),
            Some(fallback.starting("desktop duplication", Ok(Some(INFO_B)))),
        )
        .unwrap();
        assert_eq!(session.info(), INFO_B);
    }

    #[test]
    fn a_primary_that_dies_before_its_first_frame_hands_over_to_the_fallback() {
        let (primary, fallback) = (Probe::new(), Probe::new());
        let session = start_with_fallback(
            TIMEOUT,
            primary.starting("graphics capture", Err("thread ended")),
            Some(fallback.starting("desktop duplication", Ok(Some(INFO_B)))),
        )
        .unwrap();
        assert_eq!(session.info(), INFO_B);
        assert!(primary.stopped());
    }

    #[test]
    fn a_silent_primary_without_a_fallback_reports_the_attempt() {
        let primary = Probe::new();
        let error = start_with_fallback(
            TIMEOUT,
            primary.starting("graphics capture", Ok(None)),
            None,
        )
        .err()
        .expect("no session without a frame");
        let text = error.to_string();
        assert!(text.contains("graphics capture"), "{text}");
        assert!(text.contains("no frame within"), "{text}");
        assert!(primary.stopped());
    }

    #[test]
    fn two_silent_attempts_are_both_named_in_the_error() {
        let (primary, fallback) = (Probe::new(), Probe::new());
        let error = start_with_fallback(
            TIMEOUT,
            primary.starting("graphics capture", Ok(None)),
            Some(fallback.failing("desktop duplication", "no output for monitor")),
        )
        .err()
        .expect("no session without a frame");
        let text = error.to_string();
        assert!(text.contains("graphics capture"), "{text}");
        assert!(text.contains("desktop duplication"), "{text}");
        assert!(text.contains("no output for monitor"), "{text}");
        assert!(primary.stopped());
    }
}
