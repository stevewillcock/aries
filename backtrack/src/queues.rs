use crate::{Backtrack, BacktrackWith};
use std::cmp::Ordering;
use std::marker::PhantomData;

#[derive(Copy, Clone)]
struct LastBacktrack {
    next_read: usize,
    id: u64,
}

/// Very weak random number generator
fn get_rand() -> u64 {
    let num = Box::new(2);
    let num: &i32 = &num;
    let address = num as *const i32;
    address as u64
}

#[derive(Clone)]
pub struct ObsTrail<V> {
    /// a random number that attemps to uniquely identify a trail.
    /// This is use to check that a cursor is used on the right trail.
    id: u64,
    events: Vec<V>,
    backtrack_points: Vec<usize>,
    last_backtrack: Option<LastBacktrack>,
}
impl<V> Default for ObsTrail<V> {
    fn default() -> Self {
        Self::new()
    }
}
impl<V> ObsTrail<V> {
    pub fn new() -> Self {
        ObsTrail {
            id: get_rand(),
            events: Default::default(),
            backtrack_points: Default::default(),
            last_backtrack: None,
        }
    }
    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn next_slot(&self) -> TrailLoc {
        TrailLoc {
            decision_level: self.backtrack_points.len(),
            event_index: self.len(),
        }
    }

    pub fn push(&mut self, value: V) {
        self.events.push(value);
    }
    pub fn pop(&mut self) -> Option<V> {
        self.events.pop()
    }
    pub fn peek(&self) -> Option<&V> {
        self.events.last()
    }
    pub fn append<Vs: IntoIterator<Item = V>>(&mut self, values: Vs) {
        self.events.extend(values);
    }

    /// Creates a new reader for this queue
    pub fn reader(&self) -> ObsTrailCursor<V> {
        ObsTrailCursor {
            queue_id: Some(self.id),
            next_read: 0,
            last_backtrack: None,
            _phantom: Default::default(),
        }
    }

    fn backtrack_with_callback(&mut self, mut f: impl FnMut(V)) {
        let after_last = self.backtrack_points.pop().expect("No backup points left.");
        while after_last < self.events.len() {
            let ev = self.events.pop().expect("No events left");
            f(ev)
        }
        let bt_id = self.last_backtrack.as_ref().map_or(0, |bt| bt.id + 1);
        self.last_backtrack = Some(LastBacktrack {
            next_read: after_last,
            id: bt_id,
        });
    }

    pub fn num_events(&self) -> usize {
        self.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn current_decision_level(&self) -> usize {
        self.backtrack_points.len()
    }

    /// Returns a slice of all events, in chronological order.
    pub fn events(&self) -> &[V] {
        &self.events
    }

    /// Looks up the last event matching the predicate `pred`.
    /// Search goes backward in the list of event and stops when either
    ///  - no event remains
    ///  - the predicate keep_going(decision_level, event_index) returns true, where
    ///    - `decision_level` is the current decision level (going from the current one down to 0)
    ///    - `event_index` is the current event index (going from the index of the last event down to 0)
    ///
    /// # Usage
    /// ```
    /// use aries_backtrack::*;
    /// let mut q = ObsTrail::new();
    /// q.push(0); // decision_level: 0, index: 0
    /// q.push(1); // decision_level: 0, index: 1
    /// q.save_state();
    /// q.push(5);  // decision_level: 1, index: 2
    /// // look up all events for the last one that is lesser than or equal to 1
    /// let te = q.last_event_matching(|n| *n <= 1, |_, _| true).unwrap();
    /// assert_eq!(te.loc.decision_level, 0);
    /// assert_eq!(te.loc.event_index, 1);
    /// assert_eq!(*te.event, 1);
    /// // only lookup in the last decision level
    /// let te = q.last_event_matching(|n| *n <= 1, |dl, _| dl >= 1);
    /// assert!(te.is_none());
    /// ```
    pub fn last_event_matching(
        &self,
        pred: impl Fn(&V) -> bool,
        keep_going: impl Fn(usize, usize) -> bool,
    ) -> Option<TrailEvent<V>> {
        let mut decision_level = self.current_decision_level();

        for event_index in (0..self.events.len()).rev() {
            if !keep_going(decision_level, event_index) {
                return None;
            }
            let e = &self.events[event_index];
            if pred(e) {
                return Some(TrailEvent {
                    loc: TrailLoc {
                        decision_level,
                        event_index,
                    },
                    event: &self.events[event_index],
                });
            }
            if decision_level > 0 && self.backtrack_points[decision_level - 1] == event_index {
                decision_level -= 1
            }
        }
        None
    }

    /// Prints the content of the trail to standard output, specifying the decision levels.
    pub fn print(&self)
    where
        V: std::fmt::Debug,
    {
        let mut dl = 0;
        for i in 0..self.events.len() {
            print!("id: {:<4} ", i);
            if dl < self.backtrack_points.len() && self.backtrack_points[dl] == i {
                dl += 1;
                print!("dl: {:<4} ", dl);
            } else {
                print!("         ");
            }
            println!("{:?}", self.events[i]);
        }
    }
}

impl<V> Backtrack for ObsTrail<V> {
    fn save_state(&mut self) -> u32 {
        self.backtrack_points.push(self.events.len());
        self.num_saved() - 1
    }

    fn num_saved(&self) -> u32 {
        self.backtrack_points.len() as u32
    }

    fn restore_last(&mut self) {
        self.backtrack_with_callback(|_| ())
    }
}

impl<V> BacktrackWith for ObsTrail<V> {
    type Event = V;
    fn restore_last_with<F: FnMut(Self::Event)>(&mut self, callback: F) {
        self.backtrack_with_callback(callback)
    }
}

#[derive(Copy, Clone)]
pub struct TrailLoc {
    /// Decision level at which an event is located
    pub decision_level: usize,
    /// Index of an event in the event list. Also represents the number of events that occurred before it
    pub event_index: usize,
}

impl PartialEq for TrailLoc {
    fn eq(&self, other: &Self) -> bool {
        self.event_index == other.event_index
    }
}
impl Eq for TrailLoc {}
impl Ord for TrailLoc {
    fn cmp(&self, other: &Self) -> Ordering {
        self.event_index.cmp(&other.event_index)
    }
}
impl PartialOrd for TrailLoc {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Debug for TrailLoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TrailLoc(dl={}, id={}", self.decision_level, self.event_index)
    }
}

/// Represents an event and its position in a trail
pub struct TrailEvent<'a, V> {
    /// location of the event in the trail
    pub loc: TrailLoc,
    /// An event in the trail.
    /// It is a reference, that links to the queue.
    pub event: &'a V,
}

pub struct ObsTrailCursor<V> {
    queue_id: Option<u64>,
    next_read: usize,
    last_backtrack: Option<u64>,
    _phantom: PhantomData<V>,
}
impl<V> ObsTrailCursor<V> {
    /// Create a new cursor that is not bound to any queue.
    /// The cursor should only read from a single queue. This is enforced in debug mode
    /// by recording the ID of the read queue on the first read and checking that read is made
    /// on a queue with the same id on all subsequent reads.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        ObsTrailCursor {
            queue_id: None,
            next_read: 0,
            last_backtrack: None,
            _phantom: Default::default(),
        }
    }

    fn assert_matches_queue(&mut self, queue: &ObsTrail<V>) {
        if let Some(id) = self.queue_id {
            assert_eq!(id, queue.id);
        } else {
            // has not been used on any queue yet, bind this cursor to the queue
            self.queue_id = Some(queue.id);
        }
    }

    // TODO: check correctness if more than one backtrack occurred between two synchronisations
    fn sync_backtrack(&mut self, queue: &ObsTrail<V>) {
        self.assert_matches_queue(queue);

        if let Some(x) = &queue.last_backtrack {
            // a backtrack has already happened in the queue, check if we are in sync
            if self.last_backtrack != Some(x.id) {
                // we have not handled this backtrack, backtrack now if have have read some
                // cancelled output
                if self.next_read > x.next_read {
                    self.next_read = x.next_read;
                }
                self.last_backtrack = Some(x.id);
            }
        }
    }

    pub fn num_pending(&mut self, queue: &ObsTrail<V>) -> usize {
        self.sync_backtrack(queue);
        let size = queue.events.len();
        size - self.next_read
    }

    pub fn pop<'q>(&mut self, queue: &'q ObsTrail<V>) -> Option<&'q V> {
        self.sync_backtrack(queue);

        let next = self.next_read;
        if next < queue.events.len() {
            self.next_read += 1;
            Some(&queue.events[next])
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_queues() {
        let mut q = ObsTrail::new();
        q.push(0);
        q.push(1);

        q.push(5);

        let mut r1 = q.reader();
        assert_eq!(r1.pop(&q), Some(&0));
        assert_eq!(r1.pop(&q), Some(&1));
        assert_eq!(r1.pop(&q), Some(&5));
        assert_eq!(r1.pop(&q), None);

        let mut r2 = q.reader();
        assert_eq!(r2.pop(&q), Some(&0));
        assert_eq!(r2.pop(&q), Some(&1));
        assert_eq!(r2.pop(&q), Some(&5));
        assert_eq!(r2.pop(&q), None);

        q.push(2);
        assert_eq!(r1.pop(&q), Some(&2));
        assert_eq!(r2.pop(&q), Some(&2));
        assert_eq!(r1.pop(&q), None);
        assert_eq!(r2.pop(&q), None);
    }

    #[test]
    fn test_backtracks() {
        let mut q = ObsTrail::new();

        q.push(1);
        q.push(2);
        q.save_state();
        q.push(3);

        let mut r = q.reader();
        assert_eq!(r.pop(&q), Some(&1));
        assert_eq!(r.pop(&q), Some(&2));
        assert_eq!(r.pop(&q), Some(&3));

        let mut r1 = q.reader();
        let mut r2 = q.reader();
        let mut r3 = q.reader();
        assert_eq!(r1.pop(&q), Some(&1));
        assert_eq!(r1.pop(&q), Some(&2));
        assert_eq!(r1.pop(&q), Some(&3));
        assert_eq!(r2.pop(&q), Some(&1));
        assert_eq!(r2.pop(&q), Some(&2));
        assert_eq!(r3.pop(&q), Some(&1));
        q.restore_last();
        assert_eq!(r1.pop(&q), None);
        assert_eq!(r2.pop(&q), None);
        assert_eq!(r3.pop(&q), Some(&2));
        assert_eq!(r3.pop(&q), None);

        let mut r = q.reader();
        assert_eq!(r.pop(&q), Some(&1));
        assert_eq!(r.pop(&q), Some(&2));
        assert_eq!(r.pop(&q), None);

        q.save_state();
        q.push(4);
        q.restore_last();
        q.push(5);
        q.save_state();
        q.push(6);
        q.restore_last();
        assert_eq!(r.pop(&q), Some(&5));
        assert_eq!(r.pop(&q), None);
    }

    #[test]
    fn event_lookups() {
        let mut q = ObsTrail::new();

        q.push(1); // (0, 0)
        q.push(2); // (0, 1)
        q.save_state();
        q.push(3); // (1, 2)
        q.push(4); // (1, 3)
        q.save_state();
        q.push(5); // (2, 4)
        q.push(3); // (2, 5)

        let test_all =
            |n: i32, expected_pos: Option<(usize, usize)>| match q.last_event_matching(|ev| ev == &n, |_, _| true) {
                None => assert!(expected_pos.is_none()),
                Some(e) => {
                    assert_eq!(Some((e.loc.decision_level, e.loc.event_index)), expected_pos);
                    assert_eq!(*e.event, n);
                }
            };

        test_all(99, None);
        test_all(-1, None);
        test_all(1, Some((0, 0)));
        test_all(2, Some((0, 1)));
        test_all(3, Some((2, 5)));
        test_all(4, Some((1, 3)));
        test_all(5, Some((2, 4)));

        // finds the position of the event, restricting itself to the last decision level
        let test_last = |n: i32, expected_pos: Option<(usize, usize)>| {
            let last_decision_level = q.current_decision_level();
            match q.last_event_matching(|ev| ev == &n, |dl, _| dl >= last_decision_level) {
                None => assert!(expected_pos.is_none()),
                Some(e) => {
                    assert_eq!(Some((e.loc.decision_level, e.loc.event_index)), expected_pos);
                    assert_eq!(*e.event, n);
                }
            };
        };

        test_last(99, None);
        test_last(-1, None);
        test_last(1, None);
        test_last(2, None);
        test_last(3, Some((2, 5)));
        test_last(4, None);
        test_last(5, Some((2, 4)));
    }
}
