use std::cell::UnsafeCell;
use std::fmt;
use std::future::Future;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

use event_listener::{Event, EventListener};

use crate::futures::Lock;
use crate::{Mutex, MutexGuard};

const WRITER_BIT: usize = 1;
const ONE_READER: usize = 2;

/// An async reader-writer lock.
///
/// This type of lock allows multiple readers or one writer at any point in time.
///
/// The locking strategy is write-preferring, which means writers are never starved.
/// Releasing a write lock wakes the next blocked reader and the next blocked writer.
///
/// # Examples
///
/// ```
/// # futures_lite::future::block_on(async {
/// use async_lock::RwLock;
///
/// let lock = RwLock::new(5);
///
/// // Multiple read locks can be held at a time.
/// let r1 = lock.read().await;
/// let r2 = lock.read().await;
/// assert_eq!(*r1, 5);
/// assert_eq!(*r2, 5);
/// drop((r1, r2));
///
/// // Only one write lock can be held at a time.
/// let mut w = lock.write().await;
/// *w += 1;
/// assert_eq!(*w, 6);
/// # })
/// ```
pub struct RwLock<T: ?Sized> {
    /// Acquired by the writer.
    mutex: Mutex<()>,

    /// Event triggered when the last reader is dropped.
    no_readers: Event,

    /// Event triggered when the writer is dropped.
    no_writer: Event,

    /// Current state of the lock.
    ///
    /// The least significant bit (`WRITER_BIT`) is set to 1 when a writer is holding the lock or
    /// trying to acquire it.
    ///
    /// The upper bits contain the number of currently active readers. Each active reader
    /// increments the state by `ONE_READER`.
    state: AtomicUsize,

    /// The inner value.
    value: UnsafeCell<T>,
}

unsafe impl<T: Send + ?Sized> Send for RwLock<T> {}
unsafe impl<T: Send + Sync + ?Sized> Sync for RwLock<T> {}

impl<T> RwLock<T> {
    /// Creates a new reader-writer lock.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_lock::RwLock;
    ///
    /// let lock = RwLock::new(0);
    /// ```
    pub const fn new(t: T) -> RwLock<T> {
        RwLock {
            mutex: Mutex::new(()),
            no_readers: Event::new(),
            no_writer: Event::new(),
            state: AtomicUsize::new(0),
            value: UnsafeCell::new(t),
        }
    }

    /// Unwraps the lock and returns the inner value.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_lock::RwLock;
    ///
    /// let lock = RwLock::new(5);
    /// assert_eq!(lock.into_inner(), 5);
    /// ```
    pub fn into_inner(self) -> T {
        self.value.into_inner()
    }
}

impl<T: ?Sized> RwLock<T> {
    /// Attempts to acquire a read lock.
    ///
    /// If a read lock could not be acquired at this time, then [`None`] is returned. Otherwise, a
    /// guard is returned that releases the lock when dropped.
    ///
    /// # Examples
    ///
    /// ```
    /// # futures_lite::future::block_on(async {
    /// use async_lock::RwLock;
    ///
    /// let lock = RwLock::new(1);
    ///
    /// let reader = lock.read().await;
    /// assert_eq!(*reader, 1);
    ///
    /// assert!(lock.try_read().is_some());
    /// # })
    /// ```
    pub fn try_read(&self) -> Option<RwLockReadGuard<'_, T>> {
        let mut state = self.state.load(Ordering::Acquire);

        loop {
            // If there's a writer holding the lock or attempting to acquire it, we cannot acquire
            // a read lock here.
            if state & WRITER_BIT != 0 {
                return None;
            }

            // Make sure the number of readers doesn't overflow.
            if state > std::isize::MAX as usize {
                process::abort();
            }

            // Increment the number of readers.
            match self.state.compare_exchange(
                state,
                state + ONE_READER,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(RwLockReadGuard(self)),
                Err(s) => state = s,
            }
        }
    }

    /// Acquires a read lock.
    ///
    /// Returns a guard that releases the lock when dropped.
    ///
    /// Note that attempts to acquire a read lock will block if there are also concurrent attempts
    /// to acquire a write lock.
    ///
    /// # Examples
    ///
    /// ```
    /// # futures_lite::future::block_on(async {
    /// use async_lock::RwLock;
    ///
    /// let lock = RwLock::new(1);
    ///
    /// let reader = lock.read().await;
    /// assert_eq!(*reader, 1);
    ///
    /// assert!(lock.try_read().is_some());
    /// # })
    /// ```
    pub fn read(&self) -> Read<'_, T> {
        Read {
            lock: self,
            state: self.state.load(Ordering::Acquire),
            listener: None,
        }
    }

    /// Attempts to acquire a read lock with the possiblity to upgrade to a write lock.
    ///
    /// If a read lock could not be acquired at this time, then [`None`] is returned. Otherwise, a
    /// guard is returned that releases the lock when dropped.
    ///
    /// Upgradable read lock reserves the right to be upgraded to a write lock, which means there
    /// can be at most one upgradable read lock at a time.
    ///
    /// # Examples
    ///
    /// ```
    /// # futures_lite::future::block_on(async {
    /// use async_lock::{RwLock, RwLockUpgradableReadGuard};
    ///
    /// let lock = RwLock::new(1);
    ///
    /// let reader = lock.upgradable_read().await;
    /// assert_eq!(*reader, 1);
    /// assert_eq!(*lock.try_read().unwrap(), 1);
    ///
    /// let mut writer = RwLockUpgradableReadGuard::upgrade(reader).await;
    /// *writer = 2;
    /// # })
    /// ```
    pub fn try_upgradable_read(&self) -> Option<RwLockUpgradableReadGuard<'_, T>> {
        // First try grabbing the mutex.
        let lock = self.mutex.try_lock()?;

        let mut state = self.state.load(Ordering::Acquire);

        // Make sure the number of readers doesn't overflow.
        if state > std::isize::MAX as usize {
            process::abort();
        }

        // Increment the number of readers.
        loop {
            match self.state.compare_exchange(
                state,
                state + ONE_READER,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Some(RwLockUpgradableReadGuard {
                        reader: RwLockReadGuard(self),
                        reserved: lock,
                    })
                }
                Err(s) => state = s,
            }
        }
    }

    /// Attempts to acquire a read lock with the possiblity to upgrade to a write lock.
    ///
    /// Returns a guard that releases the lock when dropped.
    ///
    /// Upgradable read lock reserves the right to be upgraded to a write lock, which means there
    /// can be at most one upgradable read lock at a time.
    ///
    /// Note that attempts to acquire an upgradable read lock will block if there are concurrent
    /// attempts to acquire another upgradable read lock or a write lock.
    ///
    /// # Examples
    ///
    /// ```
    /// # futures_lite::future::block_on(async {
    /// use async_lock::{RwLock, RwLockUpgradableReadGuard};
    ///
    /// let lock = RwLock::new(1);
    ///
    /// let reader = lock.upgradable_read().await;
    /// assert_eq!(*reader, 1);
    /// assert_eq!(*lock.try_read().unwrap(), 1);
    ///
    /// let mut writer = RwLockUpgradableReadGuard::upgrade(reader).await;
    /// *writer = 2;
    /// # })
    /// ```
    pub fn upgradable_read(&self) -> UpgradableRead<'_, T> {
        UpgradableRead {
            lock: self,
            acquire: self.mutex.lock(),
        }
    }

    /// Attempts to acquire a write lock.
    ///
    /// If a write lock could not be acquired at this time, then [`None`] is returned. Otherwise, a
    /// guard is returned that releases the lock when dropped.
    ///
    /// # Examples
    ///
    /// ```
    /// # futures_lite::future::block_on(async {
    /// use async_lock::RwLock;
    ///
    /// let lock = RwLock::new(1);
    ///
    /// assert!(lock.try_write().is_some());
    /// let reader = lock.read().await;
    /// assert!(lock.try_write().is_none());
    /// # })
    /// ```
    pub fn try_write(&self) -> Option<RwLockWriteGuard<'_, T>> {
        // First try grabbing the mutex.
        let lock = self.mutex.try_lock()?;

        // If there are no readers, grab the write lock.
        if self
            .state
            .compare_exchange(0, WRITER_BIT, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            Some(RwLockWriteGuard {
                writer: RwLockWriteGuardInner(self),
                reserved: lock,
            })
        } else {
            None
        }
    }

    /// Acquires a write lock.
    ///
    /// Returns a guard that releases the lock when dropped.
    ///
    /// # Examples
    ///
    /// ```
    /// # futures_lite::future::block_on(async {
    /// use async_lock::RwLock;
    ///
    /// let lock = RwLock::new(1);
    ///
    /// let writer = lock.write().await;
    /// assert!(lock.try_read().is_none());
    /// # })
    /// ```
    pub fn write(&self) -> Write<'_, T> {
        Write {
            lock: self,
            state: WriteState::Acquiring(self.mutex.lock()),
        }
    }

    /// Returns a mutable reference to the inner value.
    ///
    /// Since this call borrows the lock mutably, no actual locking takes place. The mutable borrow
    /// statically guarantees no locks exist.
    ///
    /// # Examples
    ///
    /// ```
    /// # futures_lite::future::block_on(async {
    /// use async_lock::RwLock;
    ///
    /// let mut lock = RwLock::new(1);
    ///
    /// *lock.get_mut() = 2;
    /// assert_eq!(*lock.read().await, 2);
    /// # })
    /// ```
    pub fn get_mut(&mut self) -> &mut T {
        unsafe { &mut *self.value.get() }
    }
}

impl<T: fmt::Debug + ?Sized> fmt::Debug for RwLock<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct Locked;
        impl fmt::Debug for Locked {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("<locked>")
            }
        }

        match self.try_read() {
            None => f.debug_struct("RwLock").field("value", &Locked).finish(),
            Some(guard) => f.debug_struct("RwLock").field("value", &&*guard).finish(),
        }
    }
}

impl<T> From<T> for RwLock<T> {
    fn from(val: T) -> RwLock<T> {
        RwLock::new(val)
    }
}

impl<T: Default + ?Sized> Default for RwLock<T> {
    fn default() -> RwLock<T> {
        RwLock::new(Default::default())
    }
}

/// The future returned by [`RwLock::read`].
pub struct Read<'a, T: ?Sized> {
    /// The lock that is being acquired.
    lock: &'a RwLock<T>,

    /// The last-observed state of the lock.
    state: usize,

    /// The listener for the "no writers" event.
    listener: Option<EventListener>,
}

impl<T: ?Sized> fmt::Debug for Read<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Read { .. }")
    }
}

impl<T: ?Sized> Unpin for Read<'_, T> {}

impl<'a, T: ?Sized> Future for Read<'a, T> {
    type Output = RwLockReadGuard<'a, T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        loop {
            if this.state & WRITER_BIT == 0 {
                // Make sure the number of readers doesn't overflow.
                if this.state > std::isize::MAX as usize {
                    process::abort();
                }

                // If nobody is holding a write lock or attempting to acquire it, increment the
                // number of readers.
                match this.lock.state.compare_exchange(
                    this.state,
                    this.state + ONE_READER,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => return Poll::Ready(RwLockReadGuard(this.lock)),
                    Err(s) => this.state = s,
                }
            } else {
                // Start listening for "no writer" events.
                let load_ordering = match &mut this.listener {
                    listener @ None => {
                        *listener = Some(this.lock.no_writer.listen());

                        // Make sure there really is no writer.
                        Ordering::SeqCst
                    }

                    Some(ref mut listener) => {
                        // Wait for the writer to finish.
                        ready!(Pin::new(listener).poll(cx));
                        this.listener = None;

                        // Notify the next reader waiting in list.
                        this.lock.no_writer.notify(1);

                        // Check the state again.
                        Ordering::Acquire
                    }
                };

                // Reload the state.
                this.state = this.lock.state.load(load_ordering);
            }
        }
    }
}

/// The future returned by [`RwLock::upgradable_read`].
pub struct UpgradableRead<'a, T: ?Sized> {
    /// The lock that is being acquired.
    lock: &'a RwLock<T>,

    /// The mutex we are trying to acquire.
    acquire: Lock<'a, ()>,
}

impl<T: ?Sized> fmt::Debug for UpgradableRead<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UpgradableRead { .. }")
    }
}

impl<T: ?Sized> Unpin for UpgradableRead<'_, T> {}

impl<'a, T: ?Sized> Future for UpgradableRead<'a, T> {
    type Output = RwLockUpgradableReadGuard<'a, T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        // Acquire the mutex.
        let mutex_guard = ready!(Pin::new(&mut this.acquire).poll(cx));

        let mut state = this.lock.state.load(Ordering::Acquire);

        // Make sure the number of readers doesn't overflow.
        if state > std::isize::MAX as usize {
            process::abort();
        }

        // Increment the number of readers.
        loop {
            match this.lock.state.compare_exchange(
                state,
                state + ONE_READER,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Poll::Ready(RwLockUpgradableReadGuard {
                        reader: RwLockReadGuard(this.lock),
                        reserved: mutex_guard,
                    });
                }
                Err(s) => state = s,
            }
        }
    }
}

/// The future returned by [`RwLock::write`].
pub struct Write<'a, T: ?Sized> {
    /// The lock that is being acquired.
    lock: &'a RwLock<T>,

    /// Current state fof this future.
    state: WriteState<'a, T>,
}

enum WriteState<'a, T: ?Sized> {
    /// We are currently acquiring the inner mutex.
    Acquiring(Lock<'a, ()>),

    /// We are currently waiting for readers to finish.
    WaitingReaders {
        /// Our current write guard.
        guard: Option<RwLockWriteGuard<'a, T>>,

        /// The listener for the "no readers" event.
        listener: Option<EventListener>,
    },
}

impl<T: ?Sized> fmt::Debug for Write<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Write { .. }")
    }
}

impl<T: ?Sized> Unpin for Write<'_, T> {}

impl<'a, T: ?Sized> Future for Write<'a, T> {
    type Output = RwLockWriteGuard<'a, T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        loop {
            match &mut this.state {
                WriteState::Acquiring(lock) => {
                    // First grab the mutex.
                    let mutex_guard = ready!(Pin::new(lock).poll(cx));

                    // Set `WRITER_BIT` and create a guard that unsets it in case this future is canceled.
                    let new_state = this.lock.state.fetch_or(WRITER_BIT, Ordering::SeqCst);
                    let guard = RwLockWriteGuard {
                        writer: RwLockWriteGuardInner(this.lock),
                        reserved: mutex_guard,
                    };

                    // If we just acquired the writer lock, return it.
                    if new_state == WRITER_BIT {
                        return Poll::Ready(guard);
                    }

                    // Start waiting for the readers to finish.
                    this.state = WriteState::WaitingReaders {
                        guard: Some(guard),
                        listener: Some(this.lock.no_readers.listen()),
                    };
                }

                WriteState::WaitingReaders {
                    guard,
                    ref mut listener,
                } => {
                    let load_ordering = if listener.is_some() {
                        Ordering::Acquire
                    } else {
                        Ordering::SeqCst
                    };

                    // Check the state again.
                    if this.lock.state.load(load_ordering) == WRITER_BIT {
                        // We are the only ones holding the lock, return it.
                        return Poll::Ready(guard.take().unwrap());
                    }

                    // Wait for the readers to finish.
                    match listener {
                        None => {
                            // Register a listener.
                            *listener = Some(this.lock.no_readers.listen());
                        }

                        Some(ref mut evl) => {
                            // Wait for the readers to finish.
                            ready!(Pin::new(evl).poll(cx));
                            *listener = None;
                        }
                    };
                }
            }
        }
    }
}

/// A guard that releases the read lock when dropped.
#[clippy::has_significant_drop]
pub struct RwLockReadGuard<'a, T: ?Sized>(&'a RwLock<T>);

unsafe impl<T: Sync + ?Sized> Send for RwLockReadGuard<'_, T> {}
unsafe impl<T: Sync + ?Sized> Sync for RwLockReadGuard<'_, T> {}

impl<T: ?Sized> Drop for RwLockReadGuard<'_, T> {
    fn drop(&mut self) {
        // Decrement the number of readers.
        if self.0.state.fetch_sub(ONE_READER, Ordering::SeqCst) & !WRITER_BIT == ONE_READER {
            // If this was the last reader, trigger the "no readers" event.
            self.0.no_readers.notify(1);
        }
    }
}

impl<T: fmt::Debug + ?Sized> fmt::Debug for RwLockReadGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: fmt::Display + ?Sized> fmt::Display for RwLockReadGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: ?Sized> Deref for RwLockReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.0.value.get() }
    }
}

/// A guard that releases the upgradable read lock when dropped.
#[clippy::has_significant_drop]
pub struct RwLockUpgradableReadGuard<'a, T: ?Sized> {
    reader: RwLockReadGuard<'a, T>,
    reserved: MutexGuard<'a, ()>,
}

unsafe impl<T: Send + Sync + ?Sized> Send for RwLockUpgradableReadGuard<'_, T> {}
unsafe impl<T: Sync + ?Sized> Sync for RwLockUpgradableReadGuard<'_, T> {}

impl<'a, T: ?Sized> RwLockUpgradableReadGuard<'a, T> {
    /// Converts this guard into a writer guard.
    fn into_writer(self) -> RwLockWriteGuard<'a, T> {
        let writer = RwLockWriteGuard {
            writer: RwLockWriteGuardInner(self.reader.0),
            reserved: self.reserved,
        };
        mem::forget(self.reader);
        writer
    }

    /// Downgrades into a regular reader guard.
    ///
    /// # Examples
    ///
    /// ```
    /// # futures_lite::future::block_on(async {
    /// use async_lock::{RwLock, RwLockUpgradableReadGuard};
    ///
    /// let lock = RwLock::new(1);
    ///
    /// let reader = lock.upgradable_read().await;
    /// assert_eq!(*reader, 1);
    ///
    /// assert!(lock.try_upgradable_read().is_none());
    ///
    /// let reader = RwLockUpgradableReadGuard::downgrade(reader);
    ///
    /// assert!(lock.try_upgradable_read().is_some());
    /// # })
    /// ```
    pub fn downgrade(guard: Self) -> RwLockReadGuard<'a, T> {
        guard.reader
    }

    /// Attempts to upgrade into a write lock.
    ///
    /// If a write lock could not be acquired at this time, then [`None`] is returned. Otherwise,
    /// an upgraded guard is returned that releases the write lock when dropped.
    ///
    /// This function can only fail if there are other active read locks.
    ///
    /// # Examples
    ///
    /// ```
    /// # futures_lite::future::block_on(async {
    /// use async_lock::{RwLock, RwLockUpgradableReadGuard};
    ///
    /// let lock = RwLock::new(1);
    ///
    /// let reader = lock.upgradable_read().await;
    /// assert_eq!(*reader, 1);
    ///
    /// let reader2 = lock.read().await;
    /// let reader = RwLockUpgradableReadGuard::try_upgrade(reader).unwrap_err();
    ///
    /// drop(reader2);
    /// let writer = RwLockUpgradableReadGuard::try_upgrade(reader).unwrap();
    /// # })
    /// ```
    pub fn try_upgrade(guard: Self) -> Result<RwLockWriteGuard<'a, T>, Self> {
        // If there are no readers, grab the write lock.
        if guard
            .reader
            .0
            .state
            .compare_exchange(ONE_READER, WRITER_BIT, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            Ok(guard.into_writer())
        } else {
            Err(guard)
        }
    }

    /// Upgrades into a write lock.
    ///
    /// # Examples
    ///
    /// ```
    /// # futures_lite::future::block_on(async {
    /// use async_lock::{RwLock, RwLockUpgradableReadGuard};
    ///
    /// let lock = RwLock::new(1);
    ///
    /// let reader = lock.upgradable_read().await;
    /// assert_eq!(*reader, 1);
    ///
    /// let mut writer = RwLockUpgradableReadGuard::upgrade(reader).await;
    /// *writer = 2;
    /// # })
    /// ```
    pub fn upgrade(guard: Self) -> Upgrade<'a, T> {
        // Set `WRITER_BIT` and decrement the number of readers at the same time.
        guard
            .reader
            .0
            .state
            .fetch_sub(ONE_READER - WRITER_BIT, Ordering::SeqCst);

        // Convert into a write guard that unsets `WRITER_BIT` in case this future is canceled.
        let guard = guard.into_writer();

        Upgrade {
            guard: Some(guard),
            listener: None,
        }
    }
}

impl<T: fmt::Debug + ?Sized> fmt::Debug for RwLockUpgradableReadGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: fmt::Display + ?Sized> fmt::Display for RwLockUpgradableReadGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: ?Sized> Deref for RwLockUpgradableReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.reader.0.value.get() }
    }
}

/// The future returned by [`RwLockUpgradableReadGuard::upgrade`].
pub struct Upgrade<'a, T: ?Sized> {
    /// The guard that we are upgrading to.
    guard: Option<RwLockWriteGuard<'a, T>>,

    /// The event listener we are waiting on.
    listener: Option<EventListener>,
}

impl<T: ?Sized> fmt::Debug for Upgrade<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Upgrade").finish()
    }
}

impl<T: ?Sized> Unpin for Upgrade<'_, T> {}

impl<'a, T: ?Sized> Future for Upgrade<'a, T> {
    type Output = RwLockWriteGuard<'a, T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let guard = this
            .guard
            .as_mut()
            .expect("cannot poll future after completion");

        // If there are readers, we need to wait for them to finish.
        loop {
            let load_ordering = if this.listener.is_some() {
                Ordering::Acquire
            } else {
                Ordering::SeqCst
            };

            // See if the number of readers is zero.
            let state = guard.writer.0.state.load(load_ordering);
            if state == WRITER_BIT {
                break;
            }

            // If there are readers, wait for them to finish.
            match &mut this.listener {
                listener @ None => {
                    // Start listening for "no readers" events.
                    *listener = Some(guard.writer.0.no_readers.listen());
                }

                Some(ref mut listener) => {
                    // Wait for the readers to finish.
                    ready!(Pin::new(listener).poll(cx));
                    this.listener = None;
                }
            }
        }

        // We are done.
        Poll::Ready(this.guard.take().unwrap())
    }
}

struct RwLockWriteGuardInner<'a, T: ?Sized>(&'a RwLock<T>);

impl<T: ?Sized> Drop for RwLockWriteGuardInner<'_, T> {
    fn drop(&mut self) {
        // Unset `WRITER_BIT`.
        self.0.state.fetch_and(!WRITER_BIT, Ordering::SeqCst);
        // Trigger the "no writer" event.
        self.0.no_writer.notify(1);
    }
}

/// A guard that releases the write lock when dropped.
#[clippy::has_significant_drop]
pub struct RwLockWriteGuard<'a, T: ?Sized> {
    writer: RwLockWriteGuardInner<'a, T>,
    reserved: MutexGuard<'a, ()>,
}

unsafe impl<T: Send + ?Sized> Send for RwLockWriteGuard<'_, T> {}
unsafe impl<T: Sync + ?Sized> Sync for RwLockWriteGuard<'_, T> {}

impl<'a, T: ?Sized> RwLockWriteGuard<'a, T> {
    /// Downgrades into a regular reader guard.
    ///
    /// # Examples
    ///
    /// ```
    /// # futures_lite::future::block_on(async {
    /// use async_lock::{RwLock, RwLockWriteGuard};
    ///
    /// let lock = RwLock::new(1);
    ///
    /// let mut writer = lock.write().await;
    /// *writer += 1;
    ///
    /// assert!(lock.try_read().is_none());
    ///
    /// let reader = RwLockWriteGuard::downgrade(writer);
    /// assert_eq!(*reader, 2);
    ///
    /// assert!(lock.try_read().is_some());
    /// # })
    /// ```
    pub fn downgrade(guard: Self) -> RwLockReadGuard<'a, T> {
        // Atomically downgrade state.
        guard
            .writer
            .0
            .state
            .fetch_add(ONE_READER - WRITER_BIT, Ordering::SeqCst);

        // Trigger the "no writer" event.
        guard.writer.0.no_writer.notify(1);

        // Convert into a read guard and return.
        let new_guard = RwLockReadGuard(guard.writer.0);
        mem::forget(guard.writer); // `RwLockWriteGuardInner::drop()` should not be called!
        new_guard
    }

    /// Downgrades into an upgradable reader guard.
    ///
    /// # Examples
    ///
    /// ```
    /// # futures_lite::future::block_on(async {
    /// use async_lock::{RwLock, RwLockUpgradableReadGuard, RwLockWriteGuard};
    ///
    /// let lock = RwLock::new(1);
    ///
    /// let mut writer = lock.write().await;
    /// *writer += 1;
    ///
    /// assert!(lock.try_read().is_none());
    ///
    /// let reader = RwLockWriteGuard::downgrade_to_upgradable(writer);
    /// assert_eq!(*reader, 2);
    ///
    /// assert!(lock.try_write().is_none());
    /// assert!(lock.try_read().is_some());
    ///
    /// assert!(RwLockUpgradableReadGuard::try_upgrade(reader).is_ok())
    /// # })
    /// ```
    pub fn downgrade_to_upgradable(guard: Self) -> RwLockUpgradableReadGuard<'a, T> {
        // Atomically downgrade state.
        guard
            .writer
            .0
            .state
            .fetch_add(ONE_READER - WRITER_BIT, Ordering::SeqCst);

        // Convert into an upgradable read guard and return.
        let new_guard = RwLockUpgradableReadGuard {
            reader: RwLockReadGuard(guard.writer.0),
            reserved: guard.reserved,
        };
        mem::forget(guard.writer); // `RwLockWriteGuardInner::drop()` should not be called!
        new_guard
    }
}

impl<T: fmt::Debug + ?Sized> fmt::Debug for RwLockWriteGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: fmt::Display + ?Sized> fmt::Display for RwLockWriteGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: ?Sized> Deref for RwLockWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.writer.0.value.get() }
    }
}

impl<T: ?Sized> DerefMut for RwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.writer.0.value.get() }
    }
}
