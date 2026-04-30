//! Write-order gate for bridge transports.
//!
//! Port of TS `src/bridge/flushGate.ts`. During initial history flush or
//! transport replacement, live messages are queued so the server receives
//! `[history..., live...]` rather than interleaved writes.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlushGate<T> {
    active: bool,
    pending: Vec<T>,
}

impl<T> Default for FlushGate<T> {
    fn default() -> Self {
        Self {
            active: false,
            pending: Vec::new(),
        }
    }
}

impl<T> FlushGate<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active(&self) -> bool {
        self.active
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn start(&mut self) {
        self.active = true;
    }

    pub fn end(&mut self) -> Vec<T> {
        self.active = false;
        std::mem::take(&mut self.pending)
    }

    pub fn enqueue<I>(&mut self, items: I) -> bool
    where
        I: IntoIterator<Item = T>,
    {
        if !self.active {
            return false;
        }
        self.pending.extend(items);
        true
    }

    pub fn drop_pending(&mut self) -> usize {
        self.active = false;
        let count = self.pending.len();
        self.pending.clear();
        count
    }

    pub fn deactivate(&mut self) {
        self.active = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inactive_gate_does_not_queue() {
        let mut gate = FlushGate::new();
        assert!(!gate.enqueue([1, 2]));
        assert_eq!(gate.pending_count(), 0);
    }

    #[test]
    fn active_gate_queues_and_end_drains() {
        let mut gate = FlushGate::new();
        gate.start();
        assert!(gate.active());
        assert!(gate.enqueue([1, 2]));
        assert!(gate.enqueue([3]));
        assert_eq!(gate.pending_count(), 3);
        assert_eq!(gate.end(), vec![1, 2, 3]);
        assert!(!gate.active());
        assert_eq!(gate.pending_count(), 0);
    }

    #[test]
    fn drop_discards_and_deactivates() {
        let mut gate = FlushGate::new();
        gate.start();
        assert!(gate.enqueue(["a", "b"]));
        assert_eq!(gate.drop_pending(), 2);
        assert!(!gate.active());
        assert_eq!(gate.pending_count(), 0);
    }

    #[test]
    fn deactivate_preserves_pending_for_replacement_transport() {
        let mut gate = FlushGate::new();
        gate.start();
        assert!(gate.enqueue(["queued"]));
        gate.deactivate();
        assert!(!gate.active());
        assert_eq!(gate.pending_count(), 1);
        gate.start();
        assert_eq!(gate.end(), vec!["queued"]);
    }
}
