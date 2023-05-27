use std::cell::UnsafeCell;
use std::fmt;
use std::future::Future;
use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::task::{Context, Poll};

mod raw;

use self::raw::{RawRead, RawRwLock, RawUpgradableRead, RawUpgrade, RawWrite};
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
    /// The underlying locking implementation.
    /// Doesn't depend on `T`.
    raw: RawRwLock,

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
            raw: RawRwLock::new(),
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
    #[inline]
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
    #[inline]
    pub fn try_read(&self) -> Option<RwLockReadGuard<'_, T>> {
        if self.raw.try_read() {
            Some(RwLockReadGuard {
                lock: &self.raw,
                value: self.value.get(),
            })
        } else {
            None
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
    #[inline]
    pub fn read(&self) -> Read<'_, T> {
        Read {
            raw: self.raw.read(),
            value: self.value.get(),
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
    #[inline]
    pub fn try_upgradable_read(&self) -> Option<RwLockUpgradableReadGuard<'_, T>> {
        if self.raw.try_upgradable_read() {
            Some(RwLockUpgradableReadGuard {
                lock: &self.raw,
                value: self.value.get(),
            })
        } else {
            None
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
    #[inline]
    pub fn upgradable_read(&self) -> UpgradableRead<'_, T> {
        UpgradableRead {
            raw: self.raw.upgradable_read(),
            value: self.value.get(),
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
        if self.raw.try_write() {
            Some(RwLockWriteGuard {
                lock: &self.raw,
                value: self.value.get(),
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
            raw: self.raw.write(),
            value: self.value.get(),
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
    /// Raw read lock acquisition future, doesn't depend on `T`.
    raw: RawRead<'a>,

    /// Pointer to the value protected by the lock. Covariant in `T`.
    value: *const T,
}

unsafe impl<T: Send + Sync + ?Sized> Send for Read<'_, T> {}
unsafe impl<T: Send + Sync + ?Sized> Sync for Read<'_, T> {}

impl<T: ?Sized> fmt::Debug for Read<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Read { .. }")
    }
}

impl<T: ?Sized> Unpin for Read<'_, T> {}

impl<'a, T: ?Sized> Future for Read<'a, T> {
    type Output = RwLockReadGuard<'a, T>;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        ready!(Pin::new(&mut self.raw).poll(cx));

        Poll::Ready(RwLockReadGuard {
            lock: self.raw.lock,
            value: self.value,
        })
    }
}

/// The future returned by [`RwLock::upgradable_read`].
pub struct UpgradableRead<'a, T: ?Sized> {
    /// Raw upgradable read lock acquisition future, doesn't depend on `T`.
    raw: RawUpgradableRead<'a>,

    /// Pointer to the value protected by the lock. Invariant in `T`
    /// as the upgradable lock could provide write access.
    value: *mut T,
}

unsafe impl<T: Send + Sync + ?Sized> Send for UpgradableRead<'_, T> {}
unsafe impl<T: Send + Sync + ?Sized> Sync for UpgradableRead<'_, T> {}

impl<T: ?Sized> fmt::Debug for UpgradableRead<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UpgradableRead { .. }")
    }
}

impl<T: ?Sized> Unpin for UpgradableRead<'_, T> {}

impl<'a, T: ?Sized> Future for UpgradableRead<'a, T> {
    type Output = RwLockUpgradableReadGuard<'a, T>;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        ready!(Pin::new(&mut self.raw).poll(cx));

        Poll::Ready(RwLockUpgradableReadGuard {
            lock: self.raw.lock,
            value: self.value,
        })
    }
}

/// The future returned by [`RwLock::write`].
pub struct Write<'a, T: ?Sized> {
    /// Raw write lock acquisition future, doesn't depend on `T`.
    raw: RawWrite<'a>,

    /// Pointer to the value protected by the lock. Invariant in `T`.
    value: *mut T,
}

unsafe impl<T: Send + Sync + ?Sized> Send for Write<'_, T> {}
unsafe impl<T: Send + Sync + ?Sized> Sync for Write<'_, T> {}

impl<T: ?Sized> fmt::Debug for Write<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Write { .. }")
    }
}

impl<T: ?Sized> Unpin for Write<'_, T> {}

impl<'a, T: ?Sized> Future for Write<'a, T> {
    type Output = RwLockWriteGuard<'a, T>;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        ready!(Pin::new(&mut self.raw).poll(cx));

        Poll::Ready(RwLockWriteGuard {
            lock: self.raw.lock,
            value: self.value,
        })
    }
}

/// A guard that releases the read lock when dropped.
#[clippy::has_significant_drop]
pub struct RwLockReadGuard<'a, T: ?Sized> {
    /// Reference to underlying locking implementation.
    /// Doesn't depend on `T`.
    lock: &'a RawRwLock,

    /// Pointer to the value protected by the lock. Covariant in `T`.
    value: *const T,
}

unsafe impl<T: Sync + ?Sized> Send for RwLockReadGuard<'_, T> {}
unsafe impl<T: Sync + ?Sized> Sync for RwLockReadGuard<'_, T> {}

impl<T: ?Sized> Drop for RwLockReadGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: we are dropping a read guard.
        unsafe {
            self.lock.read_unlock();
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
        unsafe { &*self.value }
    }
}

/// A guard that releases the upgradable read lock when dropped.
#[clippy::has_significant_drop]
pub struct RwLockUpgradableReadGuard<'a, T: ?Sized> {
    /// Reference to underlying locking implementation.
    /// Doesn't depend on `T`.
    /// This guard holds a lock on the witer mutex!
    lock: &'a RawRwLock,

    /// Pointer to the value protected by the lock. Invariant in `T`
    /// as the upgradable lock could provide write access.
    value: *mut T,
}

impl<'a, T: ?Sized> Drop for RwLockUpgradableReadGuard<'a, T> {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: we are dropping an upgradable read guard.
        unsafe {
            self.lock.upgradable_read_unlock();
        }
    }
}

unsafe impl<T: Send + Sync + ?Sized> Send for RwLockUpgradableReadGuard<'_, T> {}
unsafe impl<T: Sync + ?Sized> Sync for RwLockUpgradableReadGuard<'_, T> {}

impl<'a, T: ?Sized> RwLockUpgradableReadGuard<'a, T> {
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
    #[inline]
    pub fn downgrade(guard: Self) -> RwLockReadGuard<'a, T> {
        let upgradable = ManuallyDrop::new(guard);

        // SAFETY: `guard` is an upgradable read lock.
        unsafe {
            upgradable.lock.downgrade_upgradable_read();
        };

        RwLockReadGuard {
            lock: upgradable.lock,
            value: upgradable.value,
        }
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
    #[inline]
    pub fn try_upgrade(guard: Self) -> Result<RwLockWriteGuard<'a, T>, Self> {
        // If there are no readers, grab the write lock.
        // SAFETY: `guard` is an upgradable read guard
        if unsafe { guard.lock.try_upgrade() } {
            let reader = ManuallyDrop::new(guard);

            Ok(RwLockWriteGuard {
                lock: reader.lock,
                value: reader.value,
            })
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
    #[inline]
    pub fn upgrade(guard: Self) -> Upgrade<'a, T> {
        let reader = ManuallyDrop::new(guard);

        Upgrade {
            // SAFETY: `reader` is an upgradable read guard
            raw: unsafe { reader.lock.upgrade() },
            value: reader.value,
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
        unsafe { &*self.value }
    }
}

/// The future returned by [`RwLockUpgradableReadGuard::upgrade`].
pub struct Upgrade<'a, T: ?Sized> {
    /// Raw read lock upgrade future, doesn't depend on `T`.
    raw: RawUpgrade<'a>,

    /// Pointer to the value protected by the lock. Invariant in `T`.
    value: *mut T,
}

impl<T: ?Sized> fmt::Debug for Upgrade<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Upgrade").finish()
    }
}

impl<T: ?Sized> Unpin for Upgrade<'_, T> {}

impl<'a, T: ?Sized> Future for Upgrade<'a, T> {
    type Output = RwLockWriteGuard<'a, T>;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let lock = ready!(Pin::new(&mut self.raw).poll(cx));

        Poll::Ready(RwLockWriteGuard {
            lock,
            value: self.value,
        })
    }
}

/// A guard that releases the write lock when dropped.
#[clippy::has_significant_drop]
pub struct RwLockWriteGuard<'a, T: ?Sized> {
    /// Reference to underlying locking implementation.
    /// Doesn't depend on `T`.
    /// This guard holds a lock on the witer mutex!
    lock: &'a RawRwLock,

    /// Pointer to the value protected by the lock. Invariant in `T`.
    value: *mut T,
}

unsafe impl<T: Send + ?Sized> Send for RwLockWriteGuard<'_, T> {}
unsafe impl<T: Sync + ?Sized> Sync for RwLockWriteGuard<'_, T> {}

impl<'a, T: ?Sized> Drop for RwLockWriteGuard<'a, T> {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: we are dropping a write lock
        unsafe {
            self.lock.write_unlock();
        }
    }
}

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
    #[inline]
    pub fn downgrade(guard: Self) -> RwLockReadGuard<'a, T> {
        let write = ManuallyDrop::new(guard);

        // SAFETY: `write` is a write guard
        unsafe {
            write.lock.downgrade_write();
        }

        RwLockReadGuard {
            lock: write.lock,
            value: write.value,
        }
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
    #[inline]
    pub fn downgrade_to_upgradable(guard: Self) -> RwLockUpgradableReadGuard<'a, T> {
        let write = ManuallyDrop::new(guard);

        // SAFETY: `write` is a write guard
        unsafe {
            write.lock.downgrade_to_upgradable();
        }

        RwLockUpgradableReadGuard {
            lock: write.lock,
            value: write.value,
        }
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
        unsafe { &*self.value }
    }
}

impl<T: ?Sized> DerefMut for RwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.value }
    }
}
