//! Async locking primitives.
//!
//! This crate provides two primitives:
//!
//! * [`Mutex`] - a mutual exclusion lock.
//! * [`RwLock`] - a reader-writer lock, allowing any number of readers or a single writer.

#![warn(missing_docs, missing_debug_implementations, rust_2018_idioms)]

#[doc(inline)]
pub use {
    async_mutex::{Mutex, MutexGuard},
    async_rwlock::{RwLock, RwLockReadGuard, RwLockWriteGuard},
};
