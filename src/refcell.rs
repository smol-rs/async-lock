//! source of inspiration :
//! <https://rust-lang.github.io/async-book/02_execution/03_wakeups.html>

mod raw;

use core::{
    cell::UnsafeCell,
    fmt,
    future::Future,
    ops::{Deref, DerefMut},
    ptr::NonNull,
    task::Poll,
};
use raw::{RawBorrow, RawBorrowMut, RawRefCell};

/// A single thread async mutable memory location.
///
/// This type of lock allows multiple readers or one writer at any point in time.
/// It's can be used with [`LocalExecutor`].
///
/// [`LocalExecutor`]: https://docs.rs/async-executor/latest/async_executor/struct.LocalExecutor.html
pub struct RefCell<T: ?Sized> {
    //borrow: Cell<BorrowFlag>,
    raw: RawRefCell,
    value: UnsafeCell<T>,
}
impl<T> RefCell<T> {
    /// Create a new RefCell.
    pub fn new(value: T) -> RefCell<T> {
        Self {
            raw: RawRefCell::new(),
            //borrow: Cell::new(BorrowFlag::Available),
            value: UnsafeCell::new(value),
        }
    }
}

impl<T: ?Sized> RefCell<T> {
    //TODO:
    // pub fn into_inner(self) -> T {
    //     self.value.into_inner()
    // }

    //pub fn replace(&self, t: T) -> T {}
    //pub fn replace_with<F>(&self, f: F) -> T
    //pub fn swap(&self, other: &RefCell<T>)

    /// Acquire a borrow on the wrapped value.
    ///
    /// This wait the end of the current borrow_mut.
    ///
    /// Returns a guard that releases the lock when dropped.
    pub fn borrow(&self) -> Borrow<'_, T> {
        Borrow::new(self.raw.borrow(), self.value.get())
    }

    /// Tried to acquire a borrow on the wrapped value.
    ///
    /// Return `None` instead of wait if a borrow_mut is in progress.
    pub fn try_borrow(&self) -> Option<Ref<'_, T>> {
        if self.raw.try_borrow() {
            Some(Ref {
                value: self.value.get(),
                lock: &self.raw,
            })
        } else {
            None
        }
    }

    /// Mutably borrows the wrapped value.
    /// `await` if the value is currently borrowed. For non `async` variant, use [`Self::try_borrow_mut`].
    /// TODO:
    pub fn borrow_mut(&self) -> BorrowMut<'_, T> {
        BorrowMut::new(self.raw.borrow_mut(), self.value.get())
    }

    /// Tried to acquire a borrow on the wrapped value.
    ///
    /// Return `None` instead of wait if a borrow_mut is in progress.
    pub fn try_borrow_mut(&self) -> Option<RefMut<'_, T>> {
        if self.raw.try_borrow_mut() {
            Some(RefMut {
                value: self.value.get(),
                lock: &self.raw,
            })
        } else {
            None
        }
    }
}

impl<T: fmt::Debug + ?Sized> fmt::Debug for RefCell<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct Locked;
        impl fmt::Debug for Locked {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("<locked>")
            }
        }

        match self.try_borrow() {
            None => f.debug_struct("RefCell").field("value", &Locked).finish(),
            Some(guard) => f.debug_struct("RefCell").field("value", &&*guard).finish(),
        }
    }
}
pub struct Borrow<'b, T: ?Sized> {
    value: NonNull<T>,
    raw: RawBorrow<'b>, // &'b
                        //waker: Waker,
}

impl<'x, T: ?Sized> Borrow<'x, T> {
    fn new(raw: RawBorrow<'x>, value: *mut T) -> Self {
        let value = unsafe { NonNull::new_unchecked(value) };
        Self { value, raw }
    }
}

impl<'b, T: ?Sized + 'b> Future for Borrow<'b, T> {
    type Output = Ref<'b, T>;

    fn poll(
        self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> Poll<Self::Output> {
        if self.raw.try_borrow() {
            Poll::Ready(Ref::<T> {
                lock: self.raw.lock,
                value: self.value.as_ptr(),
            })
        } else {
            // set state waiting ?
            self.raw.lock.borrow_wake(cx.waker().clone());
            Poll::Pending
        }
    }
}

impl<T: ?Sized> fmt::Debug for Borrow<'_, T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Borrow { .. }")
    }
}
pub struct BorrowMut<'b, T: ?Sized> {
    value: NonNull<T>,
    raw: RawBorrowMut<'b>, // &'b
                           //waker: Waker,
}

impl<'x, T: ?Sized> BorrowMut<'x, T> {
    fn new(raw: RawBorrowMut<'x>, value: *mut T) -> Self {
        let value = unsafe { NonNull::new_unchecked(value) };
        Self { value, raw }
    }
}

impl<'b, T: ?Sized + 'b> Future for BorrowMut<'b, T> {
    type Output = RefMut<'b, T>;

    fn poll(
        self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> Poll<Self::Output> {
        if self.raw.try_borrow_mut() {
            Poll::Ready(RefMut::<T> {
                lock: self.raw.lock,
                value: self.value.as_ptr(),
            })
        } else {
            // set state waiting ?
            self.raw.lock.borrow_mut_wake(cx.waker().clone());
            Poll::Pending
        }
    }
}

impl<T: ?Sized> fmt::Debug for BorrowMut<'_, T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BorrowMut { .. }")
    }
}

/// Wraps a borrowed reference to a value in a `RefCell` box.
/// A wrapper type for an immutably borrowed value from a `RefCell<T>`.
///
/// See the [module-level documentation](self) for more.
pub struct Ref<'a, T: ?Sized + 'a> {
    lock: &'a RawRefCell,
    value: *const T,
}

impl<T: ?Sized> Drop for Ref<'_, T> {
    #[inline]
    fn drop(&mut self) {
        self.lock.borrow_unlock();
    }
}
impl<T: fmt::Debug + ?Sized> fmt::Debug for Ref<'_, T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}
impl<T: fmt::Display + ?Sized> fmt::Display for Ref<'_, T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}
impl<T: ?Sized> Deref for Ref<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        unsafe { &*self.value }
    }
}

/// A wrapper type for a mutably borrowed value from a RefCell<T>.
///
/// See the [module-level documentation](self) for more.
pub struct RefMut<'a, T: ?Sized + 'a> {
    lock: &'a RawRefCell,
    value: *mut T,
}

impl<T: ?Sized> Drop for RefMut<'_, T> {
    #[inline]
    fn drop(&mut self) {
        self.lock.borrow_mut_unlock();
    }
}
impl<T: fmt::Debug + ?Sized> fmt::Debug for RefMut<'_, T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}
impl<T: fmt::Display + ?Sized> fmt::Display for RefMut<'_, T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}
impl<T: ?Sized> Deref for RefMut<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        unsafe { &*self.value }
    }
}
impl<T: ?Sized> DerefMut for RefMut<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.value }
    }
}
