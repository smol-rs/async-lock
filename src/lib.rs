//! Async synchronization primitives.
//!
//! This crate provides the following primitives:
//!
//! * [`Barrier`] - enables tasks to synchronize all together at the same time.
//! * [`Mutex`] - a mutual exclusion lock.
//! * [`RwLock`] - a reader-writer lock, allowing any number of readers or a single writer.
//! * [`Semaphore`] - limits the number of concurrent operations.

#![warn(missing_docs, missing_debug_implementations, rust_2018_idioms)]

use event_listener::EventListener;
use std::{future::Future, marker::PhantomData};

/// Simple macro to extract the value of `Poll` or return `Pending`.
///
/// TODO: Drop in favor of `core::task::ready`, once MSRV is bumped to 1.64.
macro_rules! ready {
    ($e:expr) => {{
        use ::core::task::Poll;

        match $e {
            Poll::Ready(v) => v,
            Poll::Pending => return Poll::Pending,
        }
    }};
}

mod barrier;
mod mutex;
mod once_cell;
mod rwlock;
mod semaphore;

pub use barrier::{Barrier, BarrierWaitResult};
pub use mutex::{Mutex, MutexGuard, MutexGuardArc};
pub use once_cell::OnceCell;
pub use rwlock::{RwLock, RwLockReadGuard, RwLockUpgradableReadGuard, RwLockWriteGuard};
pub use semaphore::{Semaphore, SemaphoreGuard, SemaphoreGuardArc};

pub mod futures {
    //! Named futures for use with `async_lock` primitives.

    pub use crate::barrier::BarrierWait;
    pub use crate::mutex::{Lock, LockArc};
    pub use crate::rwlock::{Read, UpgradableRead, Upgrade, Write};
    pub use crate::semaphore::{Acquire, AcquireArc};
}

/// The strategy for polling an `event_listener::EventListener`.
trait Strategy {
    /// The future that can be polled to wait on the listener.
    type Fut: Future<Output = ()>;

    /// The context for a poll.
    type Context: ?Sized;

    /// Polls the event listener.
    fn poll(&mut self, cx: &mut Self::Context, evl: EventListener) -> Result<(), EventListener>;

    /// A future that polls the event listener.
    fn future(&mut self, evl: EventListener) -> Self::Fut;
}

/// The strategy for blocking the current thread on an `EventListener`.
struct Blocking;

impl Strategy for Blocking {
    type Fut = std::future::Ready<()>;
    type Context = ();

    fn poll(&mut self, _cx: &mut Self::Context, evl: EventListener) -> Result<(), EventListener> {
        evl.wait();
        Ok(())
    }

    fn future(&mut self, evl: EventListener) -> Self::Fut {
        evl.wait();
        std::future::ready(())
    }
}

/// The strategy for polling an `EventListener` in an async context.
#[derive(Default)]
struct NonBlocking<'a>(PhantomData<&'a mut ()>);

impl<'a> Strategy for NonBlocking<'a> {
    type Fut = EventListener;
    type Context = std::task::Context<'a>;

    fn poll(
        &mut self,
        cx: &mut Self::Context,
        mut evl: EventListener,
    ) -> Result<(), EventListener> {
        use std::task::Poll;
        match std::pin::Pin::new(&mut evl).poll(cx) {
            Poll::Ready(()) => Ok(()),
            Poll::Pending => Err(evl),
        }
    }

    fn future(&mut self, evl: EventListener) -> Self::Fut {
        evl
    }
}
