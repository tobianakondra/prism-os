//! Fixed-capacity FIFO byte queue (pure, no I/O).
//!
//! ROLE:
//! Buffers decoded keyboard bytes between the IRQ1 handler (producer) and
//! the shell (consumer). No heap, no allocator: a 256-byte array plus head,
//! tail, and length. When full, `push` drops the newest byte and reports
//! `false` — losing a keystroke under a 256-deep backlog is the honest
//! failure mode; silently overwriting unread bytes would be worse.
//!
//! CONCURRENCY CONTRACT (enforced by the kernel, not here):
//! The producer is the IRQ1 handler, the consumer is the main loop, and an
//! IRQ can preempt the consumer MID-POP. This type is NOT internally
//! synchronized: the kernel wraps the consumer side in `without_interrupts`
//! (see `keyboard::pop_key`), which makes every pop atomic against the
//! producer. The producer needs no guard: x86 masks the IRQ while its
//! handler runs, so it can never preempt itself.
//!
//! HOST TESTS: ordering, wraparound, full/empty edges — all below.

/// Queue capacity in bytes. 256 keystrokes of backlog: a fast typist at
/// 10 keys/sec gets ~25 seconds before drops, an eternity for the shell.
pub const KEY_QUEUE_CAPACITY: usize = 256;

/// A 256-byte first-in-first-out queue. See module docs for the contract.
pub struct KeyQueue {
    buf: [u8; KEY_QUEUE_CAPACITY],
    /// Index of the next byte to pop.
    head: usize,
    /// Index where the next pushed byte lands.
    tail: usize,
    /// Bytes currently stored. The single source of truth for full/empty:
    /// `head == tail` alone is ambiguous (could mean either).
    len: usize,
}

impl KeyQueue {
    /// Empty queue.
    pub const fn new() -> Self {
        Self {
            buf: [0; KEY_QUEUE_CAPACITY],
            head: 0,
            tail: 0,
            len: 0,
        }
    }

    /// Bytes currently stored.
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when no bytes are stored.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// True when no more bytes fit.
    pub fn is_full(&self) -> bool {
        self.len == KEY_QUEUE_CAPACITY
    }

    /// Append a byte. Returns `false` (byte dropped) when full.
    pub fn push(&mut self, byte: u8) -> bool {
        if self.is_full() {
            return false;
        }
        self.buf[self.tail] = byte;
        self.tail = (self.tail + 1) % KEY_QUEUE_CAPACITY;
        self.len += 1;
        true
    }

    /// Remove and return the oldest byte, or `None` when empty.
    pub fn pop(&mut self) -> Option<u8> {
        if self.is_empty() {
            return None;
        }
        let byte = self.buf[self.head];
        self.head = (self.head + 1) % KEY_QUEUE_CAPACITY;
        self.len -= 1;
        Some(byte)
    }
}

impl Default for KeyQueue {
    /// Default = empty queue. Required by Clippy (`new_without_default`).
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_queue_pops_none() {
        let mut q = KeyQueue::new();
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
        assert_eq!(q.pop(), None);
    }

    #[test]
    fn fifo_order_is_preserved() {
        let mut q = KeyQueue::new();
        assert!(q.push(b'a'));
        assert!(q.push(b'b'));
        assert!(q.push(b'c'));
        assert_eq!(q.len(), 3);
        assert_eq!(q.pop(), Some(b'a'));
        assert_eq!(q.pop(), Some(b'b'));
        assert_eq!(q.pop(), Some(b'c'));
        assert_eq!(q.pop(), None);
        assert!(q.is_empty());
    }

    #[test]
    fn indices_wrap_around_correctly() {
        let mut q = KeyQueue::new();
        // Fill completely, drain completely: head and tail both wrap to 0.
        for i in 0..KEY_QUEUE_CAPACITY {
            assert!(q.push((i % 251) as u8));
        }
        assert!(q.is_full());
        for i in 0..KEY_QUEUE_CAPACITY {
            assert_eq!(q.pop(), Some((i % 251) as u8));
        }
        assert!(q.is_empty());
        // And the queue is fully usable again after wrapping.
        assert!(q.push(b'z'));
        assert_eq!(q.pop(), Some(b'z'));
    }

    #[test]
    fn full_queue_drops_newest_and_reports() {
        let mut q = KeyQueue::new();
        for _ in 0..KEY_QUEUE_CAPACITY {
            assert!(q.push(b'x'));
        }
        // 257th byte is dropped, oldest content untouched.
        assert!(!q.push(b'Q'));
        assert_eq!(q.len(), KEY_QUEUE_CAPACITY);
        assert_eq!(q.pop(), Some(b'x'));
        // One slot freed: push works again.
        assert!(q.push(b'y'));
    }
}
