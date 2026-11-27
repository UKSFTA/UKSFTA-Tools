//! The range queue shared by the segmented workers. It holds no IO, so the
//! retry rules can be tested without a server.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::segment::Chunk;

/// Every range is either queued, in flight, or done. A failed range returns
/// to the front, so the next worker takes it before any untouched range.
#[derive(Debug, Default)]
pub struct Scheduler {
    queue: VecDeque<Chunk>,
    in_flight: BTreeMap<u32, Chunk>,
    done: BTreeSet<u32>,
    dead_workers: usize,
}

impl Scheduler {
    pub fn new(chunks: &[Chunk]) -> Self {
        Self {
            queue: chunks.iter().copied().collect(),
            in_flight: BTreeMap::new(),
            done: BTreeSet::new(),
            dead_workers: 0,
        }
    }

    /// Take the next range. A retried range is at the front.
    pub fn take(&mut self) -> Option<Chunk> {
        let chunk = self.queue.pop_front()?;
        self.in_flight.insert(chunk.index, chunk);
        Some(chunk)
    }

    /// Mark a range as written and acknowledged.
    pub fn complete(&mut self, index: u32) {
        if self.in_flight.remove(&index).is_some() {
            self.done.insert(index);
        }
    }

    /// Return a range to the front. A range that is not in flight is ignored,
    /// so a completed range can never run twice.
    pub fn fail(&mut self, index: u32) {
        if let Some(chunk) = self.in_flight.remove(&index) {
            self.queue.push_front(chunk);
        }
    }

    /// True when every range is done and none is in flight.
    pub fn is_finished(&self) -> bool {
        self.queue.is_empty() && self.in_flight.is_empty()
    }

    /// The ranges not yet done.
    pub fn remaining(&self) -> usize {
        self.queue.len() + self.in_flight.len()
    }

    /// The acknowledged ranges.
    pub fn done(&self) -> &BTreeSet<u32> {
        &self.done
    }

    /// Mark one worker as out of attempts.
    pub fn mark_worker_dead(&mut self) {
        self.dead_workers += 1;
    }

    pub fn dead_workers(&self) -> usize {
        self.dead_workers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunks(count: u32) -> Vec<Chunk> {
        (0..count)
            .map(|index| Chunk {
                index,
                start: u64::from(index) * 4096,
                len: 4096,
            })
            .collect()
    }

    #[test]
    fn a_failed_range_returns_to_the_front_once() {
        let mut scheduler = Scheduler::new(&chunks(3));
        let first = scheduler.take().expect("a range");
        assert_eq!(first.index, 0);
        scheduler.fail(first.index);

        assert_eq!(scheduler.take().map(|chunk| chunk.index), Some(0));
        assert_eq!(scheduler.take().map(|chunk| chunk.index), Some(1));
        scheduler.complete(0);
        scheduler.fail(0);
        assert_eq!(scheduler.remaining(), 2, "a done range cannot return");
    }

    #[test]
    fn remaining_never_counts_a_done_range() {
        let mut scheduler = Scheduler::new(&chunks(3));
        assert_eq!(scheduler.remaining(), 3);
        let zero = scheduler.take().expect("a range");
        assert_eq!(scheduler.remaining(), 3, "an in-flight range is not done");
        scheduler.complete(zero.index);
        assert_eq!(scheduler.remaining(), 2);
        let one = scheduler.take().expect("a range");
        scheduler.complete(one.index);
        assert_eq!(scheduler.remaining(), 1);
        let two = scheduler.take().expect("a range");
        scheduler.complete(two.index);
        assert_eq!(scheduler.remaining(), 0);
        assert!(scheduler.is_finished());
    }

    #[test]
    fn a_range_in_flight_keeps_the_scheduler_open() {
        let mut scheduler = Scheduler::new(&chunks(1));
        let chunk = scheduler.take().expect("a range");
        assert!(!scheduler.is_finished(), "one range is in flight");
        assert_eq!(scheduler.remaining(), 1);
        scheduler.complete(chunk.index);
        assert!(scheduler.is_finished());
        assert_eq!(scheduler.done().len(), 1);
    }

    #[test]
    fn a_dead_worker_is_counted() {
        let mut scheduler = Scheduler::new(&chunks(2));
        scheduler.mark_worker_dead();
        scheduler.mark_worker_dead();
        assert_eq!(scheduler.dead_workers(), 2);
    }
}
