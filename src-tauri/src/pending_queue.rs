//! The build queue shared by `overlay.rs` and `strip.rs`: both defer window creation to
//! `RunEvent::MainEventsCleared`, because building from inside one of the event loop's own
//! dispatches never returns and freezes the app (todo 46). Insert policy stays with each
//! caller - overlay REPLACES a queued entry, strip SKIPS and reports whether it inserted.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

/// A queued entry addressed by the window label it will build.
pub trait Labeled {
    fn label(&self) -> &str;
}

pub struct PendingQueue<T> {
    queue: Mutex<Vec<T>>,
    poison_logged: AtomicBool,
    name: &'static str,
}

impl<T> PendingQueue<T> {
    /// `name` prefixes the poison log line, so the two queues stay distinguishable there.
    pub const fn new(name: &'static str) -> Self {
        Self { queue: Mutex::new(Vec::new()), poison_logged: AtomicBool::new(false), name }
    }

    /// Recovers from poison rather than skipping, the way `save_settings` does: the queue is
    /// plain data, and a skip would stop every window of this kind from ever being built
    /// again. Logged once, not per call - the drain runs on every event-loop tick.
    pub fn lock(&self) -> MutexGuard<'_, Vec<T>> {
        self.queue.lock().unwrap_or_else(|poisoned| {
            if !self.poison_logged.swap(true, Ordering::SeqCst) {
                log::error!("{} pending build queue lock poisoned, recovering", self.name);
            }
            poisoned.into_inner()
        })
    }

    /// Empties the queue, releasing the lock before the caller builds anything: `build()`
    /// creates a real WebView2 and `reconcile` runs on another thread while it does.
    pub fn take(&self) -> Vec<T> {
        std::mem::take(&mut *self.lock())
    }
}

/// Pure: drop queued builds whose label `reconcile` no longer wants.
pub fn prune_pending<T: Labeled>(queue: &mut Vec<T>, wanted_labels: &[String]) {
    queue.retain(|e| wanted_labels.iter().any(|w| w == e.label()));
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Entry(&'static str);

    impl Labeled for Entry {
        fn label(&self) -> &str {
            self.0
        }
    }

    #[test]
    fn take_hands_back_every_entry_and_leaves_the_queue_empty() {
        let q: PendingQueue<Entry> = PendingQueue::new("test");
        q.lock().push(Entry("a"));
        q.lock().push(Entry("b"));

        let taken = q.take();

        assert_eq!(taken.len(), 2);
        assert!(q.lock().is_empty());
    }

    #[test]
    fn prune_pending_keeps_only_wanted_labels() {
        let mut queue = vec![Entry("a"), Entry("b")];
        prune_pending(&mut queue, &["b".to_string()]);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].label(), "b");
    }
}
