use core::fmt;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::task::{Context, Poll};

use alloc::sync::Arc;

use event_listener::{Event, EventListener};

/// A counter for limiting the number of concurrent operations.
#[derive(Debug)]
pub struct Semaphore {
    count: AtomicUsize,
    event: Event,
}

impl Semaphore {
    /// Creates a new semaphore with a limit of `n` concurrent operations.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_lock::Semaphore;
    ///
    /// let s = Semaphore::new(5);
    /// ```
    pub const fn new(n: usize) -> Semaphore {
        Semaphore {
            count: AtomicUsize::new(n),
            event: Event::new(),
        }
    }

    /// Attempts to get a permit for a concurrent operation.
    ///
    /// If the permit could not be acquired at this time, then [`None`] is returned. Otherwise, a
    /// guard is returned that releases the mutex when dropped.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_lock::Semaphore;
    ///
    /// let s = Semaphore::new(2);
    ///
    /// let g1 = s.try_acquire().unwrap();
    /// let g2 = s.try_acquire().unwrap();
    ///
    /// assert!(s.try_acquire().is_none());
    /// drop(g2);
    /// assert!(s.try_acquire().is_some());
    /// ```
    pub fn try_acquire(&self) -> Option<SemaphoreGuard<'_>> {
        let mut count = self.count.load(Ordering::Acquire);
        loop {
            if count == 0 {
                return None;
            }

            match self.count.compare_exchange_weak(
                count,
                count - 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(SemaphoreGuard(self)),
                Err(c) => count = c,
            }
        }
    }

    /// Waits for a permit for a concurrent operation.
    ///
    /// Returns a guard that releases the permit when dropped.
    ///
    /// # Examples
    ///
    /// ```
    /// # futures_lite::future::block_on(async {
    /// use async_lock::Semaphore;
    ///
    /// let s = Semaphore::new(2);
    /// let guard = s.acquire().await;
    /// # });
    /// ```
    pub fn acquire(&self) -> Acquire<'_> {
        Acquire {
            semaphore: self,
            listener: EventListener::new(&self.event),
        }
    }

    /// Attempts to get an owned permit for a concurrent operation.
    ///
    /// If the permit could not be acquired at this time, then [`None`] is returned. Otherwise, an
    /// owned guard is returned that releases the mutex when dropped.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_lock::Semaphore;
    /// use std::sync::Arc;
    ///
    /// let s = Arc::new(Semaphore::new(2));
    ///
    /// let g1 = s.try_acquire_arc().unwrap();
    /// let g2 = s.try_acquire_arc().unwrap();
    ///
    /// assert!(s.try_acquire_arc().is_none());
    /// drop(g2);
    /// assert!(s.try_acquire_arc().is_some());
    /// ```
    pub fn try_acquire_arc(self: &Arc<Self>) -> Option<SemaphoreGuardArc> {
        let mut count = self.count.load(Ordering::Acquire);
        loop {
            if count == 0 {
                return None;
            }

            match self.count.compare_exchange_weak(
                count,
                count - 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(SemaphoreGuardArc(self.clone())),
                Err(c) => count = c,
            }
        }
    }

    /// Waits for an owned permit for a concurrent operation.
    ///
    /// Returns a guard that releases the permit when dropped.
    ///
    /// # Examples
    ///
    /// ```
    /// # futures_lite::future::block_on(async {
    /// use async_lock::Semaphore;
    /// use std::sync::Arc;
    ///
    /// let s = Arc::new(Semaphore::new(2));
    /// let guard = s.acquire_arc().await;
    /// # });
    /// ```
    pub fn acquire_arc(self: &Arc<Self>) -> AcquireArc {
        AcquireArc {
            semaphore: self.clone(),
            listener: EventListener::new(&self.event),
        }
    }

    /// Adds `n` additional permits to the semaphore.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_lock::Semaphore;
    ///
    /// # futures_lite::future::block_on(async {
    /// let s = Semaphore::new(1);
    ///
    /// let _guard = s.acquire().await;
    /// assert!(s.try_acquire().is_none());
    ///
    /// s.add_permits(2);
    ///
    /// let _guard = s.acquire().await;
    /// let _guard = s.acquire().await;
    /// # });
    /// ```
    pub fn add_permits(&self, n: usize) {
        self.count.fetch_add(n, Ordering::AcqRel);
        self.event.notify(n);
    }
}

pin_project_lite::pin_project! {
    /// The future returned by [`Semaphore::acquire`].
    pub struct Acquire<'a> {
        // The semaphore being acquired.
        semaphore: &'a Semaphore,

        // The listener waiting on the semaphore.
        #[pin]
        listener: EventListener,
    }
}

impl fmt::Debug for Acquire<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Acquire { .. }")
    }
}

impl<'a> Future for Acquire<'a> {
    type Output = SemaphoreGuard<'a>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        loop {
            match this.semaphore.try_acquire() {
                Some(guard) => return Poll::Ready(guard),
                None => {
                    // Wait on the listener.
                    if !this.listener.is_listening() {
                        this.listener.as_mut().listen();
                    } else {
                        ready!(this.listener.as_mut().poll(cx));
                    }
                }
            }
        }
    }
}

pin_project_lite::pin_project! {
    /// The future returned by [`Semaphore::acquire_arc`].
    pub struct AcquireArc {
        // The semaphore being acquired.
        semaphore: Arc<Semaphore>,

        // The listener waiting on the semaphore.
        #[pin]
        listener: EventListener,
    }
}

impl fmt::Debug for AcquireArc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AcquireArc { .. }")
    }
}

impl Future for AcquireArc {
    type Output = SemaphoreGuardArc;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        loop {
            match this.semaphore.try_acquire_arc() {
                Some(guard) => return Poll::Ready(guard),
                None => {
                    // Wait on the listener.
                    if !this.listener.is_listening() {
                        this.listener.as_mut().listen();
                    } else {
                        ready!(this.listener.as_mut().poll(cx));
                    }
                }
            }
        }
    }
}

/// A guard that releases the acquired permit.
#[clippy::has_significant_drop]
#[derive(Debug)]
pub struct SemaphoreGuard<'a>(&'a Semaphore);

impl Drop for SemaphoreGuard<'_> {
    fn drop(&mut self) {
        self.0.count.fetch_add(1, Ordering::AcqRel);
        self.0.event.notify(1);
    }
}

/// An owned guard that releases the acquired permit.
#[clippy::has_significant_drop]
#[derive(Debug)]
pub struct SemaphoreGuardArc(Arc<Semaphore>);

impl Drop for SemaphoreGuardArc {
    fn drop(&mut self) {
        self.0.count.fetch_add(1, Ordering::AcqRel);
        self.0.event.notify(1);
    }
}
