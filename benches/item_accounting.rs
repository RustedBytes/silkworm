// Measures the production accounting/notification path, not HTTP or pipeline I/O.
// Burst of 200,000 admitted guards; each producer yields after 64 completions.
// The module's unit-test helpers are unused when this benchmark has no test harness.
#[allow(dead_code)]
#[path = "../src/engine/pending.rs"]
mod pending;
use pending::PendingWorkGuard;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

async fn trial(producers: usize, total: usize) -> (f64, usize) {
    let count = Arc::new(AtomicUsize::new(0));
    let notify = Arc::new(Notify::new());
    let batches: Vec<Vec<_>> = (0..producers)
        .map(|_| {
            (0..total / producers)
                .map(|_| PendingWorkGuard::<true>::new(&count, &notify))
                .collect()
        })
        .collect();
    let start = Instant::now();
    let monitor = tokio::spawn(async move {
        let mut wakes = 0;
        loop {
            let changed = notify.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if count.load(Ordering::SeqCst) == 0 {
                return wakes;
            }
            changed.await;
            wakes += 1;
        }
    });
    let mut tasks = Vec::new();
    for batch in batches {
        tasks.push(tokio::spawn(async move {
            for (i, guard) in batch.into_iter().enumerate() {
                drop(guard);
                if i % 64 == 63 {
                    tokio::task::yield_now().await;
                }
            }
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    let wakes = monitor.await.unwrap();
    (start.elapsed().as_secs_f64() * 1000., wakes)
}
fn main() {
    for multi in [false, true] {
        let mut builder = if multi {
            tokio::runtime::Builder::new_multi_thread()
        } else {
            tokio::runtime::Builder::new_current_thread()
        };
        if multi {
            builder.worker_threads(4);
        }
        let rt = builder.enable_all().build().unwrap();
        for producers in [1, 4] {
            for _ in 0..2 {
                rt.block_on(trial(producers, 200_000));
            }
            for sample in 0..9 {
                let (ms, wakes) = rt.block_on(trial(producers, 200_000));
                println!(
                    "multi={multi} producers={producers} sample={sample} completed=200000 failures=0 ms={ms:.3} wakes={wakes}"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}
