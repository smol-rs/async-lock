//! Demonstrates fairness properties of the mutex.
//!
//! A number of threads run a loop in which they hold the lock for a little bit and re-acquire it
//! immediately after. In the end we print the number of times each thread acquired the lock.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use async_mutex::Mutex;
use smol::Timer;

fn main() {
    let num_threads = 30;
    let mut threads = Vec::new();
    let hits = Arc::new(Mutex::new(vec![0; num_threads]));

    for i in 0..num_threads {
        let hits = hits.clone();
        threads.push(thread::spawn(move || {
            smol::run(async {
                let start = Instant::now();

                while start.elapsed() < Duration::from_secs(1) {
                    let mut hits = hits.lock().await;
                    hits[i] += 1;
                    Timer::after(Duration::from_micros(5000)).await;
                }
            })
        }));
    }

    for t in threads {
        t.join().unwrap();
    }

    dbg!(hits);
}
