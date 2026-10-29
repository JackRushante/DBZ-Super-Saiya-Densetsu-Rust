//! Buffers kept from one picture to the next.
//!
//! An Enhanced picture is several megabytes, and a fresh allocation of that size is a fresh
//! mapping of the operating system's pages: the first write to each page is a fault, a few
//! milliseconds a frame at 4x. A [`Pool`] keeps the buffers of the pictures that are gone (see
//! [`crate::HdFrame`]'s drop) and hands one out again, so the pages stay mapped. The content of a
//! buffer taken is whatever it held: the caller writes every element before it reads any.

use std::sync::{Mutex, PoisonError};

/// How many buffers a pool keeps (the pictures in flight: the one shown, the one being uploaded,
/// the one being drawn, and one spare).
const KEPT: usize = 4;

pub(crate) struct Pool<T> {
    free: Mutex<Vec<Vec<T>>>,
}

impl<T: Copy + Default> Pool<T> {
    pub(crate) const fn new() -> Self {
        Self { free: Mutex::new(Vec::new()) }
    }

    /// A buffer of exactly `len` elements: a kept one when there is one that fits (its elements
    /// are those it was given back with, new ones are `T::default()`), else a new one.
    pub(crate) fn take(&self, len: usize) -> Vec<T> {
        let mut buffer = {
            let mut free = self.free.lock().unwrap_or_else(PoisonError::into_inner);
            let at = free
                .iter()
                .position(|kept| kept.len() == len)
                .or_else(|| free.iter().position(|kept| kept.capacity() >= len));
            at.map(|at| free.swap_remove(at)).unwrap_or_default()
        };
        buffer.resize(len, T::default());
        buffer
    }

    /// Gives a buffer back for [`Pool::take`] to hand out again. When the pool is full the oldest
    /// buffer is dropped.
    pub(crate) fn give(&self, buffer: Vec<T>) {
        if buffer.capacity() == 0 {
            return;
        }
        let mut free = self.free.lock().unwrap_or_else(PoisonError::into_inner);
        if free.len() >= KEPT {
            free.remove(0);
        }
        free.push(buffer);
    }
}

/// How many items one task of a pass over `items` rows takes at least: about one band for each worker
/// of the pool, so a pass is a few tasks and not a task for each row.
pub(crate) fn band(items: usize) -> usize {
    items.div_ceil(rayon::current_num_threads().max(1)).clamp(4, 32)
}

/// The bytes of the Enhanced pictures.
pub(crate) static BYTES: Pool<u8> = Pool::new();

/// The words the filters work on.
pub(crate) static WORDS: Pool<u32> = Pool::new();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_buffer_given_back_is_handed_out_again() {
        let pool: Pool<u8> = Pool::new();
        let mut first = pool.take(100);
        assert_eq!(first.len(), 100);
        first[7] = 9;
        let pointer = first.as_ptr();
        pool.give(first);
        let again = pool.take(100);
        assert_eq!(again.as_ptr(), pointer);
        pool.give(again);
        // A smaller one is cut from it, a bigger one is grown with zeros
        let small = pool.take(10);
        assert_eq!(small.len(), 10);
        pool.give(small);
        let big = pool.take(200);
        assert_eq!(big.len(), 200);
        assert!(big[10..].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn the_pool_keeps_a_few_buffers_only() {
        let pool: Pool<u32> = Pool::new();
        for len in 1..=KEPT + 3 {
            pool.give(vec![0; len]);
        }
        assert_eq!(pool.free.lock().unwrap().len(), KEPT);
    }
}
