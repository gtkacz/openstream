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

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;

    use super::*;

    #[test]
    fn put_overwrites_unread_value_and_counts_the_drop() {
        let slot = LatestSlot::new();
        slot.put(1);
        slot.put(2);
        assert_eq!(slot.try_take(), Some(2));
        assert_eq!(slot.dropped(), 1);
        assert_eq!(slot.try_take(), None);
    }

    #[test]
    fn take_blocks_until_a_value_arrives() {
        let slot = LatestSlot::new();
        let producer = {
            let slot = slot.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(20));
                slot.put(7);
            })
        };
        assert_eq!(slot.take(), Some(7));
        producer.join().unwrap();
    }

    #[test]
    fn take_timeout_reports_timeout_then_value_then_closed() {
        let slot = LatestSlot::new();
        assert!(matches!(
            slot.take_timeout(Duration::from_millis(10)),
            SlotWait::Timeout
        ));
        slot.put("x");
        assert!(matches!(
            slot.take_timeout(Duration::from_millis(10)),
            SlotWait::Value("x")
        ));
        slot.close();
        assert!(matches!(
            slot.take_timeout(Duration::from_millis(10)),
            SlotWait::Closed
        ));
        assert_eq!(slot.take(), None);
    }

    #[test]
    fn close_wakes_a_blocked_taker() {
        let slot: Arc<LatestSlot<u8>> = LatestSlot::new();
        let waiter = {
            let slot = slot.clone();
            thread::spawn(move || slot.take())
        };
        thread::sleep(Duration::from_millis(20));
        slot.close();
        assert_eq!(waiter.join().unwrap(), None);
    }
}
