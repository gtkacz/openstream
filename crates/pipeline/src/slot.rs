use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
pub struct LatestSlot<T> {
    state: Mutex<State<T>>,
    ready: Condvar,
}
struct State<T> {
    value: Option<T>,
    closed: bool,
    dropped: u64,
}
pub enum SlotWait<T> {
    Value(T),
    Timeout,
    Closed,
}
impl<T> LatestSlot<T> {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                value: None,
                closed: false,
                dropped: 0,
            }),
            ready: Condvar::new(),
        })
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
    pub fn put(&self, v: T) {
        let mut s = self.lock();
        if s.closed {
            return;
        }
        if s.value.replace(v).is_some() {
            s.dropped += 1
        }
        self.ready.notify_one()
    }
    pub fn try_take(&self) -> Option<T> {
        self.lock().value.take()
    }
    pub fn take(&self) -> Option<T> {
        let mut s = self.lock();
        loop {
            if let Some(v) = s.value.take() {
                return Some(v);
            }
            if s.closed {
                return None;
            }
            s = self.ready.wait(s).unwrap_or_else(|p| p.into_inner())
        }
    }
    pub fn take_timeout(&self, d: Duration) -> SlotWait<T> {
        let mut s = self.lock();
        if let Some(v) = s.value.take() {
            return SlotWait::Value(v);
        }
        if s.closed {
            return SlotWait::Closed;
        }
        let (mut s, r) = self
            .ready
            .wait_timeout(s, d)
            .unwrap_or_else(|p| p.into_inner());
        if let Some(v) = s.value.take() {
            SlotWait::Value(v)
        } else if s.closed {
            SlotWait::Closed
        } else {
            let _ = r;
            SlotWait::Timeout
        }
    }
    pub fn close(&self) {
        self.lock().closed = true;
        self.ready.notify_all()
    }
    pub fn dropped(&self) -> u64 {
        self.lock().dropped
    }
}
