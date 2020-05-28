//! Reference-counted async lock.
//!
//! The [`Lock`] type is similar to [`std::sync::Mutex`], except locking is an async operation.
//!
//! Note that [`Lock`] by itself acts like an [`Arc`] in the sense that cloning it returns just
//! another reference to the same lock.
//!
//! Furthermore, [`LockGuard`] is not tied to [`Lock`] by a lifetime, so you can keep guards for
//! as long as you want. This is useful when you want to spawn a task and move a guard into its
//! future.
//!
//! The locking mechanism uses eventual fairness to ensure locking will be fair on average without
//! sacrificing performance. This is done by forcing a fair lock whenever a lock operation is
//! starved for longer than 0.5 milliseconds.
//!
//! # Examples
//!
//! ```
//! # smol::run(async {
//! use async_lock::Lock;
//! use smol::Task;
//!
//! let lock = Lock::new(0);
//! let mut tasks = vec![];
//!
//! for _ in 0..10 {
//!     let lock = lock.clone();
//!     tasks.push(Task::spawn(async move { *lock.lock().await += 1 }));
//! }
//!
//! for task in tasks {
//!     task.await;
//! }
//! assert_eq!(*lock.lock().await, 10);
//! # })
//! ```

#![warn(missing_docs, missing_debug_implementations, rust_2018_idioms)]

use std::cell::UnsafeCell;
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use event_listener::Event;

/// An async lock.
pub struct Lock<T>(Arc<Inner<T>>);

impl<T> Clone for Lock<T> {
    fn clone(&self) -> Lock<T> {
        Lock(self.0.clone())
    }
}

/// Data inside [`Lock`].
struct Inner<T> {
    /// Current state of the lock.
    ///
    /// The least significant bit is set to 1 if the lock is acquired.
    /// The other bits hold the number of starved lock operations.
    state: AtomicUsize,

    /// Lock operations waiting for the lock to be released.
    lock_ops: Event,

    /// The value inside the lock.
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for Lock<T> {}
unsafe impl<T: Send> Sync for Lock<T> {}

impl<T> Lock<T> {
    /// Creates a new async lock.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_lock::Lock;
    ///
    /// let lock = Lock::new(0);
    /// ```
    pub fn new(data: T) -> Lock<T> {
        Lock(Arc::new(Inner {
            state: AtomicUsize::new(0),
            lock_ops: Event::new(),
            data: UnsafeCell::new(data),
        }))
    }

    /// Acquires the lock.
    ///
    /// Returns a guard that releases the lock when dropped.
    ///
    /// # Examples
    ///
    /// ```
    /// # smol::block_on(async {
    /// use async_lock::Lock;
    ///
    /// let lock = Lock::new(10);
    /// let guard = lock.lock().await;
    /// assert_eq!(*guard, 10);
    /// # })
    /// ```
    #[inline]
    pub async fn lock(&self) -> LockGuard<T> {
        if let Some(guard) = self.try_lock() {
            return guard;
        }
        self.lock_slow().await
    }

    /// Slow path for acquiring the lock.
    #[cold]
    pub async fn lock_slow(&self) -> LockGuard<T> {
        // Get the current time.
        let start = Instant::now();

        loop {
            // Start listening for events.
            let listener = self.0.lock_ops.listen();

            // Try locking if nobody is being starved.
            match self.0.state.compare_and_swap(0, 1, Ordering::Acquire) {
                // Lock acquired!
                0 => return LockGuard(self.clone()),

                // Lock is held and nobody is starved.
                1 => {}

                // Somebody is starved.
                _ => break,
            }

            // Wait for a notification.
            listener.await;

            // Try locking if nobody is being starved.
            match self.0.state.compare_and_swap(0, 1, Ordering::Acquire) {
                // Lock acquired!
                0 => return LockGuard(self.clone()),

                // Lock is held and nobody is starved.
                1 => {}

                // Somebody is starved.
                _ => {
                    // Notify the first listener in line because we probably received a
                    // notification that was meant for a starved task.
                    self.0.lock_ops.notify_one();
                    break;
                }
            }

            // If waiting for too long, fall back to a fairer locking strategy that will prevent
            // newer lock operations from starving us forever.
            if start.elapsed() > Duration::from_micros(500) {
                break;
            }
        }

        // Increment the number of starved lock operations.
        if self.0.state.fetch_add(2, Ordering::Release) > usize::MAX / 2 {
            // In case of potential overflow, abort.
            process::abort();
        }

        // Decrement the counter when exiting this function.
        let _call = CallOnDrop(|| {
            self.0.state.fetch_sub(2, Ordering::Release);
        });

        loop {
            // Start listening for events.
            let listener = self.0.lock_ops.listen();

            // Try locking if nobody else is being starved.
            match self.0.state.compare_and_swap(2, 2 | 1, Ordering::Acquire) {
                // Lock acquired!
                2 => return LockGuard(self.clone()),

                // Lock is held by someone.
                s if s % 2 == 1 => {}

                // Lock is available.
                _ => {
                    // Be fair: notify the first listener and then go wait in line.
                    self.0.lock_ops.notify_one();
                }
            }

            // Wait for a notification.
            listener.await;

            // Try acquiring the lock without waiting for others.
            if self.0.state.fetch_or(1, Ordering::Acquire) % 2 == 0 {
                return LockGuard(self.clone());
            }
        }
    }

    /// Attempts to acquire the lock.
    ///
    /// If the lock could not be acquired at this time, then [`None`] is returned. Otherwise, a
    /// guard is returned that releases the lock when dropped.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_lock::Lock;
    ///
    /// let lock = Lock::new(10);
    /// if let Some(guard) = lock.try_lock() {
    ///     assert_eq!(*guard, 10);
    /// }
    /// # ;
    /// ```
    #[inline]
    pub fn try_lock(&self) -> Option<LockGuard<T>> {
        if self.0.state.compare_and_swap(0, 1, Ordering::Acquire) == 0 {
            Some(LockGuard(self.clone()))
        } else {
            None
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for Lock<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct Locked;
        impl fmt::Debug for Locked {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("<locked>")
            }
        }

        match self.try_lock() {
            None => f.debug_struct("Lock").field("data", &Locked).finish(),
            Some(guard) => f.debug_struct("Lock").field("data", &&*guard).finish(),
        }
    }
}

impl<T> From<T> for Lock<T> {
    fn from(val: T) -> Lock<T> {
        Lock::new(val)
    }
}

impl<T: Default> Default for Lock<T> {
    fn default() -> Lock<T> {
        Lock::new(Default::default())
    }
}

/// A guard that releases the lock when dropped.
pub struct LockGuard<T>(Lock<T>);

unsafe impl<T: Send> Send for LockGuard<T> {}
unsafe impl<T: Sync> Sync for LockGuard<T> {}

impl<T> LockGuard<T> {
    /// Returns a reference to the lock a guard came from.
    ///
    /// # Examples
    ///
    /// ```
    /// # smol::block_on(async {
    /// use async_lock::{Lock, LockGuard};
    ///
    /// let lock = Lock::new(10i32);
    /// let guard = lock.lock().await;
    /// dbg!(LockGuard::source(&guard));
    /// # })
    /// ```
    pub fn source(guard: &LockGuard<T>) -> &Lock<T> {
        &guard.0
    }
}

impl<T> Drop for LockGuard<T> {
    fn drop(&mut self) {
        // Remove the last bit and notify a waiting lock operation.
        (self.0).0.state.fetch_sub(1, Ordering::Release);
        (self.0).0.lock_ops.notify_one();
    }
}

impl<T: fmt::Debug> fmt::Debug for LockGuard<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: fmt::Display> fmt::Display for LockGuard<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T> Deref for LockGuard<T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*(self.0).0.data.get() }
    }
}

impl<T> DerefMut for LockGuard<T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *(self.0).0.data.get() }
    }
}

/// Calls a function when dropped.
struct CallOnDrop<F: Fn()>(F);

impl<F: Fn()> Drop for CallOnDrop<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}
