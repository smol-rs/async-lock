#![feature(test)]

extern crate test;

use std::sync::Arc;
use std::thread;

use async_executor::Executor;
use futures_lite::future;
use once_cell::sync::Lazy;
use test::Bencher;
use tokio::sync::Mutex;

static EX: Lazy<Executor> = Lazy::new(|| {
    for _ in 0..num_cpus::get() {
        thread::spawn(|| future::block_on(EX.run(future::pending::<()>())));
    }
    Executor::new()
});

#[bench]
fn create(b: &mut Bencher) {
    b.iter(|| Mutex::new(()));
}

#[bench]
fn contention(b: &mut Bencher) {
    b.iter(|| future::block_on(run(10, 1000)));
}

#[bench]
fn no_contention(b: &mut Bencher) {
    b.iter(|| future::block_on(run(1, 10000)));
}

async fn run(task: usize, iter: usize) {
    let m = Arc::new(Mutex::new(()));
    let mut tasks = Vec::new();

    for _ in 0..task {
        let m = m.clone();
        tasks.push(EX.spawn(async move {
            for _ in 0..iter {
                let _ = m.lock().await;
            }
        }));
    }

    for t in tasks {
        t.await;
    }
}
