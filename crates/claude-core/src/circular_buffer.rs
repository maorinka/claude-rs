//! Fixed-size ring buffer.
//!
//! Port of TS `src/utils/CircularBuffer.ts`. Stores the most recent
//! `capacity` items; `add` overwrites the oldest slot when full.
//! `get_recent` returns the tail of the buffer in arrival order;
//! `to_array` returns the full contents oldest-first.
//!
//! A capacity of zero is allowed: `add` silently drops. This
//! matches the TS behaviour where the backing array is sized to 0
//! and the modulo `% 0` branch is never entered because `size`
//! stays at 0.

/// Fixed-capacity circular buffer.
#[derive(Debug)]
pub struct CircularBuffer<T> {
    buffer: Vec<Option<T>>,
    head: usize,
    size: usize,
    capacity: usize,
}

impl<T> CircularBuffer<T> {
    /// Create a buffer with fixed `capacity`. `capacity == 0` yields
    /// a buffer that drops every push — matching TS semantics.
    pub fn new(capacity: usize) -> Self {
        let mut buffer = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            buffer.push(None);
        }
        Self {
            buffer,
            head: 0,
            size: 0,
            capacity,
        }
    }

    /// Append a single item. Evicts the oldest when full. Drops
    /// silently when `capacity == 0`.
    pub fn add(&mut self, item: T) {
        if self.capacity == 0 {
            return;
        }
        self.buffer[self.head] = Some(item);
        self.head = (self.head + 1) % self.capacity;
        if self.size < self.capacity {
            self.size += 1;
        }
    }

    /// Append many items, oldest-first.
    pub fn add_all<I>(&mut self, items: I)
    where
        I: IntoIterator<Item = T>,
    {
        for item in items {
            self.add(item);
        }
    }

    /// Number of items currently held.
    pub fn len(&self) -> usize {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Capacity the buffer was constructed with.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Discard all items. Leaves `capacity` unchanged.
    pub fn clear(&mut self) {
        for slot in self.buffer.iter_mut() {
            *slot = None;
        }
        self.head = 0;
        self.size = 0;
    }
}

impl<T: Clone> CircularBuffer<T> {
    /// Return the most recent `count` items in arrival order (oldest
    /// of the tail first). Fewer than `count` are returned if the
    /// buffer holds less.
    pub fn get_recent(&self, count: usize) -> Vec<T> {
        if self.capacity == 0 || self.size == 0 {
            return Vec::new();
        }
        let start = if self.size < self.capacity { 0 } else { self.head };
        let available = count.min(self.size);
        let mut out = Vec::with_capacity(available);
        for i in 0..available {
            let index = (start + self.size - available + i) % self.capacity;
            if let Some(v) = &self.buffer[index] {
                out.push(v.clone());
            }
        }
        out
    }

    /// Return every item currently held, oldest-first.
    pub fn to_array(&self) -> Vec<T> {
        if self.capacity == 0 || self.size == 0 {
            return Vec::new();
        }
        let start = if self.size < self.capacity { 0 } else { self.head };
        let mut out = Vec::with_capacity(self.size);
        for i in 0..self.size {
            let index = (start + i) % self.capacity;
            if let Some(v) = &self.buffer[index] {
                out.push(v.clone());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_buffer_returns_nothing() {
        let b: CircularBuffer<i32> = CircularBuffer::new(4);
        assert!(b.is_empty());
        assert_eq!(b.len(), 0);
        assert_eq!(b.to_array(), Vec::<i32>::new());
        assert_eq!(b.get_recent(10), Vec::<i32>::new());
    }

    #[test]
    fn add_under_capacity_preserves_order() {
        let mut b = CircularBuffer::new(4);
        b.add(1);
        b.add(2);
        b.add(3);
        assert_eq!(b.len(), 3);
        assert_eq!(b.to_array(), vec![1, 2, 3]);
    }

    #[test]
    fn overflow_evicts_oldest() {
        let mut b = CircularBuffer::new(3);
        b.add_all([1, 2, 3, 4, 5]);
        assert_eq!(b.len(), 3);
        assert_eq!(b.to_array(), vec![3, 4, 5]);
    }

    #[test]
    fn get_recent_returns_tail() {
        let mut b = CircularBuffer::new(4);
        b.add_all([1, 2, 3]);
        assert_eq!(b.get_recent(2), vec![2, 3]);
        assert_eq!(b.get_recent(10), vec![1, 2, 3]);
    }

    #[test]
    fn get_recent_after_wrap() {
        let mut b = CircularBuffer::new(3);
        b.add_all([1, 2, 3, 4, 5]);
        assert_eq!(b.get_recent(2), vec![4, 5]);
        assert_eq!(b.get_recent(3), vec![3, 4, 5]);
    }

    #[test]
    fn clear_resets_everything() {
        let mut b = CircularBuffer::new(3);
        b.add_all([1, 2, 3, 4]);
        b.clear();
        assert_eq!(b.len(), 0);
        assert!(b.is_empty());
        assert_eq!(b.to_array(), Vec::<i32>::new());
        b.add(9);
        assert_eq!(b.to_array(), vec![9]);
    }

    #[test]
    fn zero_capacity_silently_drops() {
        let mut b: CircularBuffer<i32> = CircularBuffer::new(0);
        b.add(1);
        b.add(2);
        assert_eq!(b.len(), 0);
        assert_eq!(b.to_array(), Vec::<i32>::new());
        assert_eq!(b.get_recent(1), Vec::<i32>::new());
    }

    #[test]
    fn capacity_one_keeps_last() {
        let mut b = CircularBuffer::new(1);
        b.add(1);
        b.add(2);
        b.add(3);
        assert_eq!(b.to_array(), vec![3]);
    }

    #[test]
    fn holds_non_clone_types_for_add_and_len() {
        #[allow(dead_code)]
        struct NotClone(i32);
        let mut b: CircularBuffer<NotClone> = CircularBuffer::new(2);
        b.add(NotClone(1));
        b.add(NotClone(2));
        b.add(NotClone(3));
        assert_eq!(b.len(), 2);
    }
}
