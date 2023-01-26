//! Async synchronization primitives.
//!
//! This crate provides the following primitives:
//!
//! * [`Barrier`] - enables tasks to synchronize all together at the same time.
//! * [`Mutex`] - a mutual exclusion lock.
//! * [`RwLock`] - a reader-writer lock, allowing any number of readers or a single writer.
//! * [`Semaphore`] - limits the number of concurrent operations.

#![warn(missing_docs, missing_debug_implementations, rust_2018_idioms)]

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
