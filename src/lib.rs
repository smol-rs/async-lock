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
use std::mem;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use async_mutex::{Mutex, MutexGuard};

/// An async lock.
pub struct Lock<T>(Arc<Inner<T>>);

unsafe impl<T: Send> Send for Lock<T> {}
unsafe impl<T: Send> Sync for Lock<T> {}

impl<T> Clone for Lock<T> {
    fn clone(&self) -> Lock<T> {
        Lock(self.0.clone())
    }
}

/// Data inside [`Lock`].
struct Inner<T> {
    /// The inner mutex.
    mutex: Mutex<()>,

    /// The value inside the lock.
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for Inner<T> {}
unsafe impl<T: Send> Sync for Inner<T> {}

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
            mutex: Mutex::new(()),
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
        LockGuard::new(self.clone(), self.0.mutex.lock().await)
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
        self.0
            .mutex
            .try_lock()
            .map(|guard| LockGuard::new(self.clone(), guard))
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
pub struct LockGuard<T>(Lock<T>, MutexGuard<'static, ()>);

unsafe impl<T: Send> Send for LockGuard<T> {}
unsafe impl<T: Sync> Sync for LockGuard<T> {}

impl<T> LockGuard<T> {
    fn new(lock: Lock<T>, inner: MutexGuard<'_, ()>) -> LockGuard<T> {
        let inner = unsafe { mem::transmute::<MutexGuard<'_, ()>, MutexGuard<'static, ()>>(inner) };
        LockGuard(lock, inner)
    }

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
