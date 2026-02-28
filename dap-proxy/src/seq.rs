use std::sync::atomic::{AtomicI64, Ordering};

/// Sequence number allocator for injected DAP requests.
///
/// Starts at 100_000 to avoid collision with Zed's seq range (1..N).
pub struct SeqAllocator {
    counter: AtomicI64,
}

impl SeqAllocator {
    pub const START: i64 = 100_000;

    pub fn new() -> Self {
        Self {
            counter: AtomicI64::new(Self::START),
        }
    }

    pub fn next(&self) -> i64 {
        self.counter.fetch_add(1, Ordering::Relaxed)
    }

    /// Returns true if this seq was allocated by us (>= START).
    pub fn is_injected(seq: i64) -> bool {
        seq >= Self::START
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_100_000() {
        let alloc = SeqAllocator::new();
        assert_eq!(alloc.next(), 100_000);
    }

    #[test]
    fn increments() {
        let alloc = SeqAllocator::new();
        assert_eq!(alloc.next(), 100_000);
        assert_eq!(alloc.next(), 100_001);
        assert_eq!(alloc.next(), 100_002);
    }

    #[test]
    fn is_injected_boundary() {
        assert!(!SeqAllocator::is_injected(0));
        assert!(!SeqAllocator::is_injected(99_999));
        assert!(SeqAllocator::is_injected(100_000));
        assert!(SeqAllocator::is_injected(100_001));
    }
}
