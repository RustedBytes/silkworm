use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::Notify;

// Keep only accounting handles here: holding EngineState would create a cycle
// between the engine's ready queue and its queued requests.
pub(super) struct PendingWorkGuard<const IDLE_ONLY: bool = false> {
    pending: Arc<AtomicUsize>,
    notify: Arc<Notify>,
}

impl<const IDLE_ONLY: bool> PendingWorkGuard<IDLE_ONLY> {
    pub(super) fn new(pending: &Arc<AtomicUsize>, notify: &Arc<Notify>) -> Self {
        pending.fetch_add(1, Ordering::SeqCst);
        Self {
            pending: pending.clone(),
            notify: notify.clone(),
        }
    }
}

impl<const IDLE_ONLY: bool> Drop for PendingWorkGuard<IDLE_ONLY> {
    fn drop(&mut self) {
        let previous = self.pending.fetch_sub(1, Ordering::SeqCst);
        // Items only affect the idle predicate. Request completions also wake
        // the coordinator to reap finished delayed tasks, so retain those wakes.
        if !IDLE_ONLY || previous == 1 {
            self.notify.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::{Future, poll_fn};
    use std::task::Poll;

    async fn notifications_match_completion_policy<const IDLE_ONLY: bool>() {
        let pending = Arc::new(AtomicUsize::new(0));
        let notify = Arc::new(Notify::new());
        let first = PendingWorkGuard::<IDLE_ONLY>::new(&pending, &notify);
        let last = PendingWorkGuard::<IDLE_ONLY>::new(&pending, &notify);
        let changed = notify.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        drop(first);
        assert_eq!(pending.load(Ordering::SeqCst), 1);
        let ready = poll_fn(|cx| Poll::Ready(changed.as_mut().poll(cx).is_ready())).await;
        assert_eq!(ready, !IDLE_ONLY);
        if ready {
            changed.set(notify.notified());
            changed.as_mut().enable();
        }
        drop(last);
        assert_eq!(pending.load(Ordering::SeqCst), 0);
        tokio::time::timeout(std::time::Duration::from_secs(5), changed)
            .await
            .expect("last completion must wake idle waiter");
    }

    #[tokio::test]
    async fn items_only_notify_on_idle_and_requests_keep_completion_notifications() {
        notifications_match_completion_policy::<true>().await;
        notifications_match_completion_policy::<false>().await;
    }

    async fn concurrent_completion_does_not_miss_idle() {
        for _ in 0..100 {
            let pending = Arc::new(AtomicUsize::new(0));
            let notify = Arc::new(Notify::new());
            let guards: Vec<_> = (0..32)
                .map(|_| PendingWorkGuard::<true>::new(&pending, &notify))
                .collect();
            let count = pending.clone();
            let waiter = tokio::spawn(async move {
                loop {
                    let changed = notify.notified();
                    tokio::pin!(changed);
                    // Register before checking the predicate: completion may race
                    // either step on a multi-thread runtime.
                    changed.as_mut().enable();
                    if count.load(Ordering::SeqCst) == 0 {
                        return;
                    }
                    changed.await;
                }
            });
            let mut tasks = tokio::task::JoinSet::new();
            for guard in guards {
                tasks.spawn(async move {
                    tokio::task::yield_now().await;
                    drop(guard);
                });
            }
            tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
                .await
                .expect("idle waiter hung")
                .expect("idle waiter panicked");
            while let Some(result) = tasks.join_next().await {
                result.expect("completion task panicked");
            }
            assert_eq!(pending.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn current_thread_completions_wake_idle_waiter() {
        concurrent_completion_does_not_miss_idle().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn multi_thread_completions_wake_idle_waiter() {
        concurrent_completion_does_not_miss_idle().await;
    }
}
