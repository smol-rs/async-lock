use std::thread;
use std::{sync::Arc, task::Poll};

use async_lock::{Mutex, PollMutex};
use future::poll_fn;
use futures_lite::future;

#[test]
fn smoke() {
    future::block_on(async {
        let m = Mutex::new(());
        drop(m.lock().await);
        drop(m.lock().await);
    })
}

#[test]
fn try_lock() {
    let m = Mutex::new(());
    *m.try_lock().unwrap() = ();
}

#[test]
fn into_inner() {
    let m = Mutex::new(10i32);
    assert_eq!(m.into_inner(), 10);
}

#[test]
fn get_mut() {
    let mut m = Mutex::new(10i32);
    *m.get_mut() = 20;
    assert_eq!(m.into_inner(), 20);
}

#[test]
fn contention() {
    future::block_on(async {
        let (tx, rx) = async_channel::unbounded();

        let tx = Arc::new(tx);
        let mutex = Arc::new(Mutex::new(0i32));
        let num_tasks = 100;

        for _ in 0..num_tasks {
            let tx = tx.clone();
            let mutex = mutex.clone();

            thread::spawn(|| {
                future::block_on(async move {
                    let mut lock = mutex.lock().await;
                    *lock += 1;
                    tx.send(()).await.unwrap();
                    drop(lock);
                })
            });
        }

        for _ in 0..num_tasks {
            rx.recv().await.unwrap();
        }

        let lock = mutex.lock().await;
        assert_eq!(num_tasks, *lock);
    });
}

#[test]
fn poll_lock() {
    future::block_on(async {
        poll_fn(|cx| -> Poll<()> {
            let m = Arc::new(Mutex::new(()));
            let mut m1 = PollMutex::new(m.clone());
            let mut m2 = PollMutex::new(m.clone());
            let guard = m1.poll_lock(cx);
            if let Poll::Ready(guard) = guard {
                assert!(m2.poll_lock(cx).is_pending());
                drop(guard);
                assert!(m2.poll_lock(cx).is_ready());
            } else {
                unreachable!();
            }
            Poll::Ready(())
        })
        .await;
    })
}

#[test]
fn poll_lock_arc() {
    future::block_on(async {
        poll_fn(|cx| -> Poll<()> {
            let m = Arc::new(Mutex::new(()));
            let mut m1 = PollMutex::new(m.clone());
            let mut m2 = PollMutex::new(m.clone());
            let guard = m1.poll_lock(cx);
            if let Poll::Ready(guard) = guard {
                assert!(m2.poll_lock_arc(cx).is_pending());
                drop(guard);
                assert!(m2.poll_lock_arc(cx).is_ready());
            } else {
                unreachable!();
            }
            Poll::Ready(())
        })
        .await;
    })
}

#[test]
fn contention_poll_lock_arc() {
    future::block_on(async {
        let (tx, rx) = async_channel::unbounded();

        let tx = Arc::new(tx);
        let mutex = Arc::new(Mutex::new(0i32));
        let num_tasks = 100;

        for _ in 0..num_tasks {
            let tx = tx.clone();
            let mutex = mutex.clone();
            let mut mutex = PollMutex::new(mutex);

            thread::spawn(|| {
                future::block_on(async move {
                    let mut lock = future::poll_fn(|cx| mutex.poll_lock_arc(cx)).await;
                    *lock += 1;
                    tx.send(()).await.unwrap();
                    drop(lock);
                })
            });
        }

        for _ in 0..num_tasks {
            rx.recv().await.unwrap()
        }

        let lock = mutex.lock().await;
        assert_eq!(num_tasks, *lock);
    });
}
