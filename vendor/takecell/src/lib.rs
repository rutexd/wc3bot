//! Minimal vendored replacement for the `takecell` crate (upstream 0.1.2
//! declares `rust-version = 1.96`, which is too new for our toolchain).
//!
//! Only the API actually used by `teloxide-core` is implemented:
//! [`TakeCell`] — a thread-safe cell whose value can be taken by reference
//! exactly once.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

/// A cell type which value can only be taken once.
///
/// Taking the value returns `&mut T`; any subsequent attempt (from any thread)
/// returns `None` until the cell is healed via [`TakeCell::heal`].
pub struct TakeCell<T: ?Sized> {
    taken: AtomicBool,
    value: UnsafeCell<T>,
}

// `&TakeCell<T>` hands out `&mut T`, so sharing it across threads requires T: Send,
// and moving it to another thread requires T: Send as well.
unsafe impl<T: ?Sized + Send> Sync for TakeCell<T> {}
unsafe impl<T: ?Sized + Send> Send for TakeCell<T> {}

impl<T> TakeCell<T> {
    pub const fn new(value: T) -> Self {
        Self {
            taken: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    /// Unwraps the underlying value, consuming this cell.
    pub fn into_inner(self) -> T {
        self.value.into_inner()
    }
}

impl<T: ?Sized> TakeCell<T> {
    /// Takes the value, returning an exclusive reference to it.
    ///
    /// Returns `None` if the value was already taken.
    pub fn take(&self) -> Option<&mut T> {
        self.taken
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| unsafe { &mut *self.value.get() })
    }

    /// Returns `true` if the value was already taken.
    pub fn is_taken(&self) -> bool {
        self.taken.load(Ordering::Acquire)
    }

    /// Returns an exclusive reference to the underlying value.
    ///
    /// The cell does not need to be untaken; after this call it stays taken.
    pub fn get(&mut self) -> &mut T {
        self.taken.store(true, Ordering::Release);
        unsafe { &mut *self.value.get() }
    }

    /// Makes the cell untaken, allowing the value to be taken again.
    ///
    /// # Safety
    /// The caller must ensure that no references previously returned by
    /// [`take`](TakeCell::take)/[`get`](TakeCell::get) are alive.
    pub unsafe fn heal(&mut self) {
        self.taken.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn take_once() {
        let cell = TakeCell::new(42);
        assert!(!cell.is_taken());
        assert_eq!(cell.take(), Some(&mut 42));
        assert!(cell.is_taken());
        assert_eq!(cell.take(), None);
    }

    #[test]
    fn get_and_heal() {
        let mut cell = TakeCell::new(String::from("hi"));
        assert_eq!(cell.get(), &"hi".to_string());
        assert!(cell.is_taken());

        unsafe { cell.heal() };
        assert!(!cell.is_taken());
        assert_eq!(cell.take().map(|s| s.as_str()), Some("hi"));
    }

    #[test]
    fn into_inner() {
        let cell = TakeCell::new(vec![1, 2, 3]);
        assert_eq!(cell.into_inner(), vec![1, 2, 3]);
    }

    #[test]
    fn shared_across_threads() {
        let cell = Arc::new(TakeCell::new(7));
        let c2 = Arc::clone(&cell);

        let first = std::thread::spawn(move || c2.take().copied());
        let won_local = cell.take().is_some();
        let won_remote = first.join().unwrap().is_some();

        // Exactly one of the two threads gets the value.
        assert_ne!(won_local, won_remote);
        assert!(cell.is_taken());
    }

    #[test]
    fn dyn_compatible_behind_arc() {
        // Mirrors teloxide-core usage: unsized coercion behind Arc.
        let cell: Arc<TakeCell<dyn std::io::Read + Send + Unpin>> =
            Arc::new(TakeCell::new(std::io::empty()));
        assert!(cell.take().is_some());
        assert!(cell.take().is_none());
    }
}
