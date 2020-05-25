use std::sync::Arc;
use std::thread;

use async_lock::Lock;
use futures::channel::mpsc;
use futures::executor::block_on;
use futures::prelude::*;

#[test]
fn smoke() {
    block_on(async {
        let l = Lock::new(());
        drop(l.lock().await);
        drop(l.lock().await);
    })
}

#[test]
fn try_lock() {
    let l = Lock::new(());
    *l.try_lock().unwrap() = ();
}

#[test]
fn contention() {
    block_on(async {
        let (tx, mut rx) = mpsc::unbounded();

        let tx = Arc::new(tx);
        let l = Lock::new(0i32);
        let num_tasks = 100;

        for _ in 0..num_tasks {
            let tx = tx.clone();
            let l = l.clone();

            thread::spawn(|| {
                block_on(async move {
                    let mut guard = l.lock().await;
                    *guard += 1;
                    tx.unbounded_send(()).unwrap();
                    drop(l);
                })
            });
        }

        for _ in 0..num_tasks {
            rx.next().await.unwrap();
        }

        let guard = l.lock().await;
        assert_eq!(num_tasks, *guard);
    });
}
