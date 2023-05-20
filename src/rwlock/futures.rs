use std::fmt;
use std::future::Future;
use std::mem::ManuallyDrop;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use super::raw::{RawRead, RawUpgradableRead, RawUpgrade, RawWrite};
use super::{
    RwLock, RwLockReadGuard, RwLockReadGuardArc, RwLockUpgradableReadGuard,
    RwLockUpgradableReadGuardArc, RwLockWriteGuard, RwLockWriteGuardArc,
};

pin_project_lite::pin_project! {
    /// The future returned by [`RwLock::read`].
    pub struct Read<'a, T: ?Sized> {
        // Raw read lock acquisition future, doesn't depend on `T`.
        #[pin]
        pub(super) raw: RawRead<'a>,

        // Pointer to the value protected by the lock. Covariant in `T`.
        pub(super) value: *const T,
    }
}

unsafe impl<T: Sync + ?Sized> Send for Read<'_, T> {}
unsafe impl<T: Sync + ?Sized> Sync for Read<'_, T> {}

impl<T: ?Sized> fmt::Debug for Read<'_, T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Read { .. }")
    }
}

impl<'a, T: ?Sized> Future for Read<'a, T> {
    type Output = RwLockReadGuard<'a, T>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        ready!(this.raw.as_mut().poll(cx));

        Poll::Ready(RwLockReadGuard {
            lock: this.raw.lock,
            value: *this.value,
        })
    }
}

pin_project_lite::pin_project! {
    /// The future returned by [`RwLock::read_arc`].
    pub struct ReadArc<'a, T> {
        // Raw read lock acquisition future, doesn't depend on `T`.
        #[pin]
        pub(super) raw: RawRead<'a>,

        // FIXME: Could be covariant in T
        pub(super) lock: &'a Arc<RwLock<T>>,
    }
}

unsafe impl<T: Send + Sync> Send for ReadArc<'_, T> {}
unsafe impl<T: Send + Sync> Sync for ReadArc<'_, T> {}

impl<T> fmt::Debug for ReadArc<'_, T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ReadArc { .. }")
    }
}

impl<'a, T> Future for ReadArc<'a, T> {
    type Output = RwLockReadGuardArc<T>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        ready!(this.raw.as_mut().poll(cx));

        // SAFETY: we just acquired a read lock
        Poll::Ready(unsafe { RwLockReadGuardArc::from_arc(this.lock.clone()) })
    }
}

pin_project_lite::pin_project! {
    /// The future returned by [`RwLock::upgradable_read`].
    pub struct UpgradableRead<'a, T: ?Sized> {
        // Raw upgradable read lock acquisition future, doesn't depend on `T`.
        #[pin]
        pub(super) raw: RawUpgradableRead<'a>,

        // Pointer to the value protected by the lock. Invariant in `T`
        // as the upgradable lock could provide write access.
        pub(super) value: *mut T,
    }
}

unsafe impl<T: Send + Sync + ?Sized> Send for UpgradableRead<'_, T> {}
unsafe impl<T: Sync + ?Sized> Sync for UpgradableRead<'_, T> {}

impl<T: ?Sized> fmt::Debug for UpgradableRead<'_, T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UpgradableRead { .. }")
    }
}

impl<'a, T: ?Sized> Future for UpgradableRead<'a, T> {
    type Output = RwLockUpgradableReadGuard<'a, T>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        ready!(this.raw.as_mut().poll(cx));

        Poll::Ready(RwLockUpgradableReadGuard {
            lock: this.raw.lock,
            value: *this.value,
        })
    }
}

pin_project_lite::pin_project! {
    /// The future returned by [`RwLock::upgradable_read_arc`].
    pub struct UpgradableReadArc<'a, T: ?Sized> {
        // Raw upgradable read lock acquisition future, doesn't depend on `T`.
        #[pin]
        pub(super) raw: RawUpgradableRead<'a>,

        pub(super) lock: &'a Arc<RwLock<T>>,
    }
}

unsafe impl<T: Send + Sync + ?Sized> Send for UpgradableReadArc<'_, T> {}
unsafe impl<T: Send + Sync + ?Sized> Sync for UpgradableReadArc<'_, T> {}

impl<T: ?Sized> fmt::Debug for UpgradableReadArc<'_, T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UpgradableReadArc { .. }")
    }
}

impl<'a, T: ?Sized> Future for UpgradableReadArc<'a, T> {
    type Output = RwLockUpgradableReadGuardArc<T>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        ready!(this.raw.as_mut().poll(cx));
        Poll::Ready(RwLockUpgradableReadGuardArc {
            lock: this.lock.clone(),
        })
    }
}

pin_project_lite::pin_project! {
    /// The future returned by [`RwLock::write`].
    pub struct Write<'a, T: ?Sized> {
        // Raw write lock acquisition future, doesn't depend on `T`.
        #[pin]
        pub(super) raw: RawWrite<'a>,

        // Pointer to the value protected by the lock. Invariant in `T`.
        pub(super) value: *mut T,
    }
}

unsafe impl<T: Send + ?Sized> Send for Write<'_, T> {}
unsafe impl<T: Sync + ?Sized> Sync for Write<'_, T> {}

impl<T: ?Sized> fmt::Debug for Write<'_, T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Write { .. }")
    }
}

impl<'a, T: ?Sized> Future for Write<'a, T> {
    type Output = RwLockWriteGuard<'a, T>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        ready!(this.raw.as_mut().poll(cx));

        Poll::Ready(RwLockWriteGuard {
            lock: this.raw.lock,
            value: *this.value,
        })
    }
}

pin_project_lite::pin_project! {
    /// The future returned by [`RwLock::write_arc`].
    pub struct WriteArc<'a, T: ?Sized> {
        // Raw write lock acquisition future, doesn't depend on `T`.
        #[pin]
        pub(super) raw: RawWrite<'a>,

        pub(super) lock: &'a Arc<RwLock<T>>,
    }
}

unsafe impl<T: Send + Sync + ?Sized> Send for WriteArc<'_, T> {}
unsafe impl<T: Send + Sync + ?Sized> Sync for WriteArc<'_, T> {}

impl<T: ?Sized> fmt::Debug for WriteArc<'_, T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WriteArc { .. }")
    }
}

impl<'a, T: ?Sized> Future for WriteArc<'a, T> {
    type Output = RwLockWriteGuardArc<T>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        ready!(this.raw.as_mut().poll(cx));

        Poll::Ready(RwLockWriteGuardArc {
            lock: this.lock.clone(),
        })
    }
}

pin_project_lite::pin_project! {
    /// The future returned by [`RwLockUpgradableReadGuard::upgrade`].
    pub struct Upgrade<'a, T: ?Sized> {
        // Raw read lock upgrade future, doesn't depend on `T`.
        #[pin]
        pub(super) raw: RawUpgrade<'a>,

        // Pointer to the value protected by the lock. Invariant in `T`.
        pub(super) value: *mut T,
    }
}

unsafe impl<T: Send + ?Sized> Send for Upgrade<'_, T> {}
unsafe impl<T: Sync + ?Sized> Sync for Upgrade<'_, T> {}

impl<T: ?Sized> fmt::Debug for Upgrade<'_, T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Upgrade").finish()
    }
}

impl<'a, T: ?Sized> Future for Upgrade<'a, T> {
    type Output = RwLockWriteGuard<'a, T>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        let lock = ready!(this.raw.as_mut().poll(cx));

        Poll::Ready(RwLockWriteGuard {
            lock,
            value: *this.value,
        })
    }
}

pin_project_lite::pin_project! {
    /// The future returned by [`RwLockUpgradableReadGuardArc::upgrade`].
    pub struct UpgradeArc<T: ?Sized> {
        // Raw read lock upgrade future, doesn't depend on `T`.
        // `'static` is a lie, this field is actually referencing the
        // `Arc` data. But since this struct also stores said `Arc`, we know
        // this value will be alive as long as the struct is.
        //
        // Yes, one field of the `ArcUpgrade` struct is referencing another.
        // Such self-references are usually not sound without pinning.
        // However, in this case, there is an indirection via the heap;
        // moving the `ArcUpgrade` won't move the heap allocation of the `Arc`,
        // so the reference inside `RawUpgrade` isn't invalidated.
        #[pin]
        pub(super) raw: ManuallyDrop<RawUpgrade<'static>>,

        // Pointer to the value protected by the lock. Invariant in `T`.
        pub(super) lock: ManuallyDrop<Arc<RwLock<T>>>,
    }

    impl<T: ?Sized> PinnedDrop for UpgradeArc<T> {
        fn drop(this: Pin<&mut Self>) {
            let this = this.project();
            if !this.raw.is_ready() {
                // SAFETY: we drop the `Arc` (decrementing the reference count)
                // only if this future was cancelled before returning an
                // upgraded lock.
                unsafe {
                    // SAFETY: The drop impl for raw assumes that it is pinned.
                    ManuallyDrop::drop(this.raw.get_unchecked_mut());
                    ManuallyDrop::drop(this.lock);
                };
            }
        }
    }
}

impl<T: ?Sized> fmt::Debug for UpgradeArc<T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArcUpgrade").finish()
    }
}

impl<T: ?Sized> Future for UpgradeArc<T> {
    type Output = RwLockWriteGuardArc<T>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        unsafe {
            // SAFETY: Practically, this is a pin projection.
            ready!(Pin::new_unchecked(&mut **this.raw.get_unchecked_mut()).poll(cx));
        }

        Poll::Ready(RwLockWriteGuardArc {
            lock: unsafe { ManuallyDrop::take(this.lock) },
        })
    }
}
