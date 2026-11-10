//! A log of events kept in memory until the owner's daily email reports them
//! (design.md, Admin). A restart empties it.

use std::collections::VecDeque;
use std::sync::Mutex;

/// The most events a log holds; past it the oldest go, and are only counted.
pub const MAX_EVENTS: usize = 1_000;

/// Events in the order they happened, each with a sequence number, so a reader can
/// drop exactly the events it read once it has reported them.
#[derive(Debug)]
pub struct EventLog<T>(Mutex<Events<T>>);

#[derive(Debug)]
struct Events<T> {
    /// The next event's sequence number.
    next: u64,
    kept: VecDeque<(u64, T)>,
    /// Every event numbered below it has been reported.
    reported: u64,
}

impl<T> Events<T> {
    /// The unreported events no longer kept.
    fn dropped(&self) -> u64 {
        let first_kept = self.kept.front().map_or(self.next, |(seq, _)| *seq);
        first_kept.saturating_sub(self.reported)
    }
}

/// What a read found: the events kept and how many more were dropped, all of them
/// numbered below `unread`.
#[derive(Debug, Clone)]
pub struct Read<T> {
    pub events: Vec<T>,
    pub dropped: u64,
    /// The sequence number of the first event recorded after the read.
    unread: u64,
}

impl<T> Read<T> {
    /// How many events happened, the dropped ones included.
    pub fn count(&self) -> u64 {
        self.events.len() as u64 + self.dropped
    }
}

impl<T> Default for EventLog<T> {
    fn default() -> Self {
        EventLog(Mutex::new(Events {
            next: 0,
            kept: VecDeque::new(),
            reported: 0,
        }))
    }
}

impl<T: Clone> EventLog<T> {
    pub fn record(&self, event: T) {
        let mut events = self.lock();
        let seq = events.next;
        events.next += 1;
        if events.kept.len() == MAX_EVENTS {
            events.kept.pop_front();
        }
        events.kept.push_back((seq, event));
    }

    /// Every unreported event, oldest first.
    pub fn read(&self) -> Read<T> {
        let events = self.lock();
        Read {
            events: events.kept.iter().map(|(_, event)| event.clone()).collect(),
            dropped: events.dropped(),
            unread: events.next,
        }
    }

    /// Marks the events `read` returned or counted as reported, keeping any recorded
    /// since.
    pub fn drop_read(&self, read: &Read<T>) {
        let mut events = self.lock();
        events.reported = events.reported.max(read.unread);
        events.kept.retain(|(seq, _)| *seq >= read.unread);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Events<T>> {
        // A panic while recording leaves nothing half-written worth refusing.
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
