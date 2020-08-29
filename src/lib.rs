//! Async synchronization primitives.
//!
//! This crate provides the following primitives:
//!
//! * [`Barrier`] - enables tasks to synchronize all together at the same time.
//! * [`Mutex`] - a mutual exclusion lock.
//! * [`RwLock`] - a reader-writer lock, allowing any number of readers or a single writer.
//! * [`Semaphore`] - limits the number of concurrent operations.

#![warn(missing_docs, missing_debug_implementations, rust_2018_idioms)]

#[doc(inline)]
pub use {async_barrier::*, async_mutex::*, async_rwlock::*, async_semaphore::*};
