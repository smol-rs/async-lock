//! Async synchronization primitives.
//!
//! This crate provides the following primitives:
//!
//! * [`Barrier`] - enables tasks to synchronize all together at the same time.
//! * [`Mutex`] - a mutual exclusion lock.
//! * [`RwLock`] - a reader-writer lock, allowing any number of readers or a single writer.
//! * [`Semaphore`] - limits the number of concurrent operations.
//! 
//! # Features
//! 
//! - `stable_deref_trait` - Uses the [`stable_deref_trait`] crate to implement the [`StableDeref`] trait
//!   for the guards of the synchronization primitives. This allows these types to be used in crates like
//!   [`owning_ref`].
//! 
//! [`stable_deref_trait`]: https://crates.io/crates/stable_deref_trait
//! [`StableDeref`]: https://docs.rs/stable_deref_trait/latest/stable_deref_trait/trait.StableDeref.html
//! [`owning_ref`]: https://crates.io/crates/owning_ref

#![warn(missing_docs, missing_debug_implementations, rust_2018_idioms)]

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
