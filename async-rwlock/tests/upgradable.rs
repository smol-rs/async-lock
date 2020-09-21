use std::sync::Arc;

use async_rwlock::{RwLock, RwLockUpgradableReadGuard};
use futures_lite::future;

#[test]
fn upgrade() {
    future::block_on(async {
        let lock: RwLock<i32> = RwLock::new(0);

        let read_guard = lock.read().await;
        let read_guard2 = lock.read().await;
        // Should be able to obtain an upgradable lock.
        let upgradable_guard = lock.upgradable_read().await;
        // Should be able to obtain a read lock when an upgradable lock is active.
        let read_guard3 = lock.read().await;
        assert_eq!(0, *read_guard3);
        drop(read_guard);
        drop(read_guard2);
        drop(read_guard3);

        // Writers should not pass.
        assert!(lock.try_write().is_none());

        let mut write_guard = RwLockUpgradableReadGuard::try_upgrade(upgradable_guard).expect(
            "should be able to upgrade an upgradable lock because there are no more readers",
        );
        *write_guard += 1;
        drop(write_guard);

        let read_guard = lock.read().await;
        assert_eq!(1, *read_guard)
    });
}

#[test]
fn not_upgrade() {
    future::block_on(async {
        let mutex: RwLock<i32> = RwLock::new(0);

        let read_guard = mutex.read().await;
        let read_guard2 = mutex.read().await;
        // Should be able to obtain an upgradable lock.
        let upgradable_guard = mutex.upgradable_read().await;
        // Should be able to obtain a shared lock when an upgradable lock is active.
        let read_guard3 = mutex.read().await;
        assert_eq!(0, *read_guard3);
        drop(read_guard);
        drop(read_guard2);
        drop(read_guard3);

        // Drop the upgradable lock.
        drop(upgradable_guard);

        assert_eq!(0, *(mutex.read().await));

        // Should be able to acquire a write lock because there are no more readers.
        let mut write_guard = mutex.write().await;
        *write_guard += 1;
        drop(write_guard);

        let read_guard = mutex.read().await;
        assert_eq!(1, *read_guard)
    });
}

#[test]
fn upgradable_with_concurrent_writer() {
    future::block_on(async {
        let lock: Arc<RwLock<i32>> = Arc::new(RwLock::new(0));
        let lock2 = lock.clone();

        let upgradable_guard = lock.upgradable_read().await;

        future::or(
            async move {
                let mut write_guard = lock2.write().await;
                *write_guard = 1;
            },
            async move {
                let mut write_guard = RwLockUpgradableReadGuard::upgrade(upgradable_guard).await;
                assert_eq!(*write_guard, 0);
                *write_guard = 2;
            },
        )
        .await;

        assert_eq!(2, *(lock.write().await));

        let read_guard = lock.read().await;
        assert_eq!(2, *read_guard);
    });
}
