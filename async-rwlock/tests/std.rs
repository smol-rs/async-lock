//! These tests were borrowed from `std::sync::RwLock`.

use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

use async_rwlock::RwLock;
use futures_lite::{future, FutureExt};

fn spawn<T: Send + 'static>(f: impl Future<Output = T> + Send + 'static) -> future::Boxed<T> {
    let (s, r) = async_channel::bounded(1);
    thread::spawn(move || {
        future::block_on(async {
            let _ = s.send(f.await).await;
        })
    });
    async move { r.recv().await.unwrap() }.boxed()
}

#[test]
fn smoke() {
    future::block_on(async {
        let lock = RwLock::new(());
        drop(lock.read().await);
        drop(lock.write().await);
        drop((lock.read().await, lock.read().await));
        drop(lock.write().await);
    });
}

#[test]
fn try_write() {
    future::block_on(async {
        let lock = RwLock::new(0isize);
        let read_guard = lock.read().await;
        assert!(lock.try_write().is_none());
        drop(read_guard);
    });
}

#[test]
fn into_inner() {
    let lock = RwLock::new(10);
    assert_eq!(lock.into_inner(), 10);
}

#[test]
fn into_inner_and_drop() {
    struct Counter(Arc<AtomicUsize>);

    impl Drop for Counter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let cnt = Arc::new(AtomicUsize::new(0));
    let lock = RwLock::new(Counter(cnt.clone()));
    assert_eq!(cnt.load(Ordering::SeqCst), 0);

    {
        let _inner = lock.into_inner();
        assert_eq!(cnt.load(Ordering::SeqCst), 0);
    }

    assert_eq!(cnt.load(Ordering::SeqCst), 1);
}

#[test]
fn get_mut() {
    let mut lock = RwLock::new(10);
    *lock.get_mut() = 20;
    assert_eq!(lock.into_inner(), 20);
}

#[test]
fn contention() {
    const N: u32 = 10;
    const M: usize = 1000;

    let (tx, rx) = async_channel::unbounded();
    let tx = Arc::new(tx);
    let rw = Arc::new(RwLock::new(()));

    // Spawn N tasks that randomly acquire the lock M times.
    for _ in 0..N {
        let tx = tx.clone();
        let rw = rw.clone();

        spawn(async move {
            for _ in 0..M {
                if fastrand::u32(..N) == 0 {
                    drop(rw.write().await);
                } else {
                    drop(rw.read().await);
                }
            }
            tx.send(()).await.unwrap();
        });
    }

    future::block_on(async move {
        for _ in 0..N {
            rx.recv().await.unwrap();
        }
    });
}

#[test]
fn writer_and_readers() {
    let lock = Arc::new(RwLock::new(0i32));
    let (tx, rx) = async_channel::unbounded();

    // Spawn a writer task.
    spawn({
        let lock = lock.clone();
        async move {
            let mut lock = lock.write().await;
            for _ in 0..1000 {
                let tmp = *lock;
                *lock = -1;
                future::yield_now().await;
                *lock = tmp + 1;
            }
            tx.send(()).await.unwrap();
        }
    });

    // Readers try to catch the writer in the act.
    let mut readers = Vec::new();
    for _ in 0..5 {
        let lock = lock.clone();
        readers.push(spawn(async move {
            for _ in 0..1000 {
                let lock = lock.read().await;
                assert!(*lock >= 0);
            }
        }));
    }

    future::block_on(async move {
        // Wait for readers to pass their asserts.
        for r in readers {
            r.await;
        }

        // Wait for writer to finish.
        rx.recv().await.unwrap();
        let lock = lock.read().await;
        assert_eq!(*lock, 1000);
    });
}
