mod pending;
mod seen;

use pending::PendingWorkGuard;
use seen::SeenRequests;

use std::cmp::Ordering as CmpOrdering;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use tokio::sync::{Mutex as AsyncMutex, Notify, mpsc, watch};
use tokio::task::JoinSet;

use crate::errors::{SilkwormError, SilkwormResult};
use crate::http::HttpClient;
use crate::logging::{Logger, complete_logs, get_logger};
use crate::middlewares::{RequestMiddleware, ResponseAction, ResponseMiddleware};
use crate::pipelines::ItemPipeline;
use crate::request::{Request, SpiderOutput};
use crate::response::Response;
use crate::spider::Spider;
use crate::types::Item;

struct Stats {
    start_time: OnceLock<Instant>,
    requests_sent: AtomicUsize,
    responses_received: AtomicUsize,
    items_scraped: AtomicUsize,
    errors: AtomicUsize,
}

impl Stats {
    fn new() -> Self {
        Stats {
            start_time: OnceLock::new(),
            requests_sent: AtomicUsize::new(0),
            responses_received: AtomicUsize::new(0),
            items_scraped: AtomicUsize::new(0),
            errors: AtomicUsize::new(0),
        }
    }

    fn set_start_time(&self, when: Instant) {
        let _ = self.start_time.set(when);
    }

    fn elapsed(&self) -> Duration {
        self.start_time
            .get()
            .map(Instant::elapsed)
            .unwrap_or_default()
    }
}

#[cfg(target_os = "linux")]
fn memory_usage_bytes() -> Option<(u64, u64)> {
    let data = std::fs::read_to_string("/proc/self/status").ok()?;
    let mut rss_kb = None;
    let mut vms_kb = None;
    for line in data.lines() {
        if line.starts_with("VmRSS:") {
            rss_kb = parse_kb(line);
        } else if line.starts_with("VmSize:") {
            vms_kb = parse_kb(line);
        }
        if rss_kb.is_some() && vms_kb.is_some() {
            break;
        }
    }
    let rss = rss_kb?.saturating_mul(1024);
    let vms = vms_kb?.saturating_mul(1024);
    Some((rss, vms))
}

#[cfg(not(target_os = "linux"))]
fn memory_usage_bytes() -> Option<(u64, u64)> {
    None
}

#[cfg(target_os = "linux")]
fn parse_kb(line: &str) -> Option<u64> {
    let mut iter = line.split_whitespace();
    let _label = iter.next()?;
    let value = iter.next()?.parse::<u64>().ok()?;
    Some(value)
}

fn format_requests_per_second(requests_sent: usize, elapsed: Duration) -> String {
    let elapsed_millis = elapsed.as_millis();
    if elapsed_millis == 0 {
        return "0.00".to_string();
    }

    let requests = u64::try_from(requests_sent).unwrap_or(u64::MAX);
    let centi_per_second = u128::from(requests)
        .saturating_mul(100_000)
        .checked_div(elapsed_millis)
        .unwrap_or(0);
    let whole = centi_per_second / 100;
    let frac = centi_per_second % 100;
    format!("{whole}.{frac:02}")
}

fn format_mebibytes(bytes: u64) -> String {
    const MIB: u128 = 1024 * 1024;
    let centi_mib = u128::from(bytes)
        .saturating_mul(100)
        .checked_div(MIB)
        .unwrap_or(0);
    let whole = centi_mib / 100;
    let frac = centi_mib % 100;
    format!("{whole}.{frac:02}")
}

#[derive(Clone)]
pub struct Engine<S: Spider> {
    state: Arc<EngineState<S>>,
}

pub struct EngineConfig<S: Spider> {
    pub concurrency: usize,
    pub request_middlewares: Vec<Arc<dyn RequestMiddleware<S>>>,
    pub response_middlewares: Vec<Arc<dyn ResponseMiddleware<S>>>,
    pub item_pipelines: Vec<Arc<dyn ItemPipeline<S>>>,
    pub request_timeout: Option<Duration>,
    pub log_stats_interval: Option<Duration>,
    /// Hard limit for ready and delayed requests, excluding active workers.
    /// Exceeding a configured limit aborts the crawl with a capacity error.
    pub max_pending_requests: Option<usize>,
    pub max_seen_requests: Option<usize>,
    pub html_max_size_bytes: usize,
    pub keep_alive: bool,
    pub fail_fast: bool,
}

struct QueuedRequest<S: Spider> {
    request: Request<S>,
    priority: i32,
    sequence: u64,
    pending_guard: Option<PendingWorkGuard>,
    queue_slot: Option<QueueSlot>,
}

impl<S: Spider> QueuedRequest<S> {
    fn new(request: Request<S>, sequence: u64) -> Self {
        QueuedRequest {
            priority: request.priority,
            request,
            sequence,
            pending_guard: None,
            queue_slot: None,
        }
    }
}

impl<S: Spider> PartialEq for QueuedRequest<S> {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.sequence == other.sequence
    }
}

impl<S: Spider> Eq for QueuedRequest<S> {}

impl<S: Spider> PartialOrd for QueuedRequest<S> {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl<S: Spider> Ord for QueuedRequest<S> {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        match self.priority.cmp(&other.priority) {
            CmpOrdering::Equal => other.sequence.cmp(&self.sequence),
            order => order,
        }
    }
}

struct EngineState<S: Spider> {
    spider: Arc<S>,
    http: HttpClient,
    queue_budget: Option<Arc<QueueBudget>>,
    item_tx: mpsc::Sender<QueuedItem>,
    item_rx: AsyncMutex<Option<mpsc::Receiver<QueuedItem>>>,
    ready_queue: AsyncMutex<BinaryHeap<QueuedRequest<S>>>,
    ready_notify: Notify,
    enqueue_sequence: AtomicU64,
    seen: AsyncMutex<SeenRequests>,
    seen_count: AtomicUsize,
    stop: AtomicBool,
    stop_tx: watch::Sender<bool>,
    stop_rx: watch::Receiver<bool>,
    pending: Arc<AtomicUsize>,
    pending_notify: Arc<Notify>,
    item_pending: Arc<AtomicUsize>,
    item_pending_notify: Arc<Notify>,
    request_middlewares: Vec<Arc<dyn RequestMiddleware<S>>>,
    response_middlewares: Vec<Arc<dyn ResponseMiddleware<S>>>,
    item_pipelines: Vec<Arc<dyn ItemPipeline<S>>>,
    log_stats_interval: Option<Duration>,
    stats: Stats,
    logger: Logger,
    html_max_size_bytes: usize,
    fail_fast: bool,
    fatal_error: AsyncMutex<Option<SilkwormError>>,
    fatal_error_notify: Notify,
    scheduled_requests: AsyncMutex<JoinSet<()>>,
}

struct QueueBudget {
    limit: usize,
    waiting: AtomicUsize,
}

struct QueueSlot(Arc<QueueBudget>);

impl QueueBudget {
    fn reserve(self: &Arc<Self>) -> Option<QueueSlot> {
        self.waiting
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |waiting| {
                (waiting < self.limit).then(|| waiting + 1)
            })
            .ok()
            .map(|_| QueueSlot(self.clone()))
    }
}

impl Drop for QueueSlot {
    fn drop(&mut self) {
        self.0.waiting.fetch_sub(1, Ordering::SeqCst);
    }
}

struct QueuedItem {
    item: Item,
    pending_guard: PendingWorkGuard<true>,
}

fn signal_stop_state<S: Spider>(state: &EngineState<S>) {
    state.stop.store(true, Ordering::SeqCst);
    let _ = state.stop_tx.send(true);
}

async fn record_fatal_error_state<S: Spider>(state: &EngineState<S>, err: SilkwormError) {
    let mut slot = state.fatal_error.lock().await;
    if slot.is_none() {
        *slot = Some(err);
        drop(slot);
        state.fatal_error_notify.notify_waiters();
    }
}

fn next_queue_sequence<S: Spider>(state: &EngineState<S>) -> u64 {
    state.enqueue_sequence.fetch_add(1, Ordering::SeqCst)
}

fn request_fingerprint<S>(req: &Request<S>) -> String {
    let method = req.method.trim().to_ascii_uppercase();
    let merged_url = crate::http::canonical_url_with_params(req.url.as_str(), &req.params)
        .unwrap_or_else(|_| req.url.clone());
    format!("{method} {merged_url}")
}

fn take_retry_delay<S>(req: &mut Request<S>) -> Option<Duration> {
    req.take_retry_delay_secs().map(Duration::from_secs_f64)
}

fn take_request_delay<S>(req: &mut Request<S>) -> Option<Duration> {
    req.take_request_delay_secs().map(Duration::from_secs_f64)
}

impl<S: Spider> Engine<S> {
    /// Build a new engine with validated runtime configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when configuration is invalid or the HTTP client cannot
    /// be created.
    pub fn new(spider: S, config: EngineConfig<S>) -> SilkwormResult<Self> {
        let EngineConfig {
            concurrency,
            request_middlewares,
            response_middlewares,
            item_pipelines,
            request_timeout,
            log_stats_interval,
            max_pending_requests,
            max_seen_requests,
            html_max_size_bytes,
            keep_alive,
            fail_fast,
        } = config;
        if max_seen_requests == Some(0) {
            return Err(SilkwormError::Config(
                "max_seen_requests must be greater than zero when set".to_string(),
            ));
        }
        if max_pending_requests == Some(0) {
            return Err(SilkwormError::Config(
                "max_pending_requests must be greater than zero when set".to_string(),
            ));
        }
        let queue_size = max_pending_requests
            .unwrap_or(concurrency.saturating_mul(10))
            .max(1);
        let (item_tx, item_rx) = mpsc::channel(queue_size);
        let (stop_tx, stop_rx) = watch::channel(false);
        let http = HttpClient::new(
            concurrency,
            crate::types::Headers::new(),
            request_timeout,
            html_max_size_bytes,
            true,
            10,
            keep_alive,
        )?;
        let spider = Arc::new(spider);
        let logger = get_logger("engine", Some(spider.name()));
        let state = EngineState {
            spider,
            http,
            queue_budget: max_pending_requests.map(|limit| {
                Arc::new(QueueBudget {
                    limit,
                    waiting: AtomicUsize::new(0),
                })
            }),
            item_tx,
            item_rx: AsyncMutex::new(Some(item_rx)),
            ready_queue: AsyncMutex::new(BinaryHeap::new()),
            ready_notify: Notify::new(),
            enqueue_sequence: AtomicU64::new(0),
            seen: AsyncMutex::new(SeenRequests::new(max_seen_requests)),
            seen_count: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
            stop_tx,
            stop_rx,
            pending: Arc::new(AtomicUsize::new(0)),
            pending_notify: Arc::new(Notify::new()),
            item_pending: Arc::new(AtomicUsize::new(0)),
            item_pending_notify: Arc::new(Notify::new()),
            request_middlewares,
            response_middlewares,
            item_pipelines,
            log_stats_interval,
            stats: Stats::new(),
            logger,
            html_max_size_bytes,
            fail_fast,
            fatal_error: AsyncMutex::new(None),
            fatal_error_notify: Notify::new(),
            scheduled_requests: AsyncMutex::new(JoinSet::new()),
        };
        Ok(Engine {
            state: Arc::new(state),
        })
    }

    /// Run the crawl engine until all work is complete or a fatal error occurs.
    ///
    /// # Errors
    ///
    /// Returns an error when startup, request processing, worker coordination,
    /// or shutdown steps fail.
    pub async fn run(&self) -> SilkwormResult<()> {
        self.state.logger.info(
            "Starting engine",
            &[("spider", self.state.spider.name().to_string())],
        );
        self.state.stats.set_start_time(Instant::now());

        let mut item_worker = self.spawn_item_worker().await?;

        let mut join_set = JoinSet::new();
        for _ in 0..self.state.http.concurrency {
            let engine = Self {
                state: self.state.clone(),
            };
            join_set.spawn(async move {
                engine.worker().await;
            });
        }

        let stats_task = if let Some(interval) = self.state.log_stats_interval
            && interval > Duration::ZERO
        {
            let engine = Self {
                state: self.state.clone(),
            };
            Some(tokio::spawn(async move {
                engine.log_statistics(interval).await;
            }))
        } else {
            None
        };

        let mut run_error: Option<SilkwormError> = None;

        if let Err(err) = self.open_spider().await {
            self.state
                .logger
                .error("Failed to open spider", &[("error", err.to_string())]);
            run_error = Some(self.take_fatal_error().await.unwrap_or(err));
        } else if let Err(err) = self
            .await_idle_or_worker_health(&mut join_set, &item_worker)
            .await
        {
            self.state
                .logger
                .error("Engine stopped unexpectedly", &[("error", err.to_string())]);
            run_error = Some(err);
        }

        self.shutdown();
        self.shutdown_scheduled_requests().await;
        self.state.ready_queue.lock().await.clear();

        if run_error.is_none() {
            if let Some(err) = self.join_workers(&mut join_set).await {
                run_error = Some(err);
            }
        } else {
            let _ = self.join_workers(&mut join_set).await;
        }

        if let Some(err) = self.join_item_worker(&mut item_worker).await
            && run_error.is_none()
        {
            run_error = Some(err);
        }

        if let Some(handle) = stats_task
            && let Err(err) = handle.await
        {
            self.state
                .logger
                .error("Statistics task failed", &[("error", format!("{err}"))]);
            if run_error.is_none() {
                run_error = Some(SilkwormError::Spider(format!(
                    "statistics task failed: {err}"
                )));
            }
        }

        if run_error.is_none() {
            self.state
                .logger
                .info("Final crawl statistics", &self.stats_payload());
        }

        self.state.http.close();

        if let Err(err) = self.close_spider().await
            && run_error.is_none()
        {
            run_error = Some(err);
        }

        complete_logs();
        if let Some(err) = run_error {
            return Err(err);
        }
        Ok(())
    }

    async fn open_spider(&self) -> SilkwormResult<()> {
        self.state.logger.info("Opening spider", &[]);
        self.state.spider.open().await;
        for pipe in &self.state.item_pipelines {
            pipe.open(self.state.spider.clone()).await?;
        }
        let requests = self.state.spider.start_requests().await;
        for req in requests {
            self.enqueue(req).await?;
        }
        Ok(())
    }

    async fn close_spider(&self) -> SilkwormResult<()> {
        self.state.logger.info("Closing spider", &[]);
        let mut first_error = None;
        for (idx, pipe) in self.state.item_pipelines.iter().enumerate() {
            if let Err(err) = pipe.close(self.state.spider.clone()).await {
                self.state.logger.error(
                    "Failed to close pipeline",
                    &[("index", idx.to_string()), ("error", err.to_string())],
                );
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
        self.state.spider.close().await;
        if let Some(err) = first_error {
            return Err(err);
        }
        Ok(())
    }

    fn try_queue_slot(&self) -> Result<Option<QueueSlot>, String> {
        match &self.state.queue_budget {
            None => Ok(None),
            Some(budget) => budget.reserve().map(Some).ok_or_else(|| format!(
                "max_pending_requests capacity ({}) exceeded; increase the limit or reduce spider fan-out",
                budget.limit,
            )),
        }
    }

    async fn check_queue_slot(
        &self,
        slot: Result<Option<QueueSlot>, String>,
    ) -> SilkwormResult<Option<QueueSlot>> {
        match slot {
            Ok(slot) => Ok(slot),
            Err(message) => {
                // Workers also produce requests. Waiting for capacity here can
                // deadlock all consumers, so overload must be an explicit error.
                record_fatal_error_state(
                    self.state.as_ref(),
                    SilkwormError::Spider(message.clone()),
                )
                .await;
                self.shutdown();
                Err(SilkwormError::Spider(message))
            }
        }
    }

    async fn enqueue(&self, mut req: Request<S>) -> SilkwormResult<()> {
        if self.state.stop.load(Ordering::SeqCst) {
            return Err(SilkwormError::Spider("Engine is stopping".to_string()));
        }
        let retry_delay = take_retry_delay(&mut req);
        let slot = if req.dont_filter {
            self.try_queue_slot()
        } else {
            let fingerprint = request_fingerprint(&req);
            let mut seen = self.state.seen.lock().await;
            if seen.contains(fingerprint.as_str()) {
                self.state
                    .logger
                    .debug("Skipping already seen request", &[("url", req.url.clone())]);
                return Ok(());
            }
            let slot = self.try_queue_slot();
            if slot.is_ok() {
                seen.insert_if_new(&fingerprint);
                self.state.seen_count.fetch_add(1, Ordering::SeqCst);
            }
            slot
        };
        let slot = self.check_queue_slot(slot).await?;
        if let Some(delay) = retry_delay {
            return self.enqueue_delayed_with_slot(req, delay, slot).await;
        }
        let queued = self.track_request(req, slot);
        self.enqueue_immediate(queued).await
    }

    fn track_request(&self, req: Request<S>, slot: Option<QueueSlot>) -> QueuedRequest<S> {
        let mut queued = QueuedRequest::new(req, next_queue_sequence(self.state.as_ref()));
        queued.pending_guard = Some(PendingWorkGuard::new(
            &self.state.pending,
            &self.state.pending_notify,
        ));
        queued.queue_slot = slot;
        queued
    }

    async fn enqueue_immediate(&self, queued: QueuedRequest<S>) -> SilkwormResult<()> {
        self.state
            .logger
            .debug("Enqueued request", &[("url", queued.request.url.clone())]);
        let mut ready = self.state.ready_queue.lock().await;
        if self.state.stop.load(Ordering::SeqCst) {
            return Err(SilkwormError::Spider("Engine is stopping".to_string()));
        }
        ready.push(queued);
        drop(ready);
        self.state.ready_notify.notify_one();
        Ok(())
    }

    async fn enqueue_delayed(&self, req: Request<S>, delay: Duration) -> SilkwormResult<()> {
        let slot = self.check_queue_slot(self.try_queue_slot()).await?;
        self.enqueue_delayed_with_slot(req, delay, slot).await
    }

    async fn enqueue_delayed_with_slot(
        &self,
        req: Request<S>,
        delay: Duration,
        slot: Option<QueueSlot>,
    ) -> SilkwormResult<()> {
        self.state.logger.debug(
            "Scheduled delayed request",
            &[
                ("url", req.url.clone()),
                ("delay_seconds", format!("{:.3}", delay.as_secs_f64())),
            ],
        );
        let mut queued = self.track_request(req, slot);
        let engine = Self {
            state: self.state.clone(),
        };
        let mut stop_rx = self.state.stop_rx.clone();
        let mut scheduled = self.state.scheduled_requests.lock().await;
        if self.state.stop.load(Ordering::SeqCst) {
            return Err(SilkwormError::Spider("Engine is stopping".to_string()));
        }
        while let Some(result) = scheduled.try_join_next() {
            if let Err(err) = result {
                self.state.logger.error(
                    "Scheduled request task failed",
                    &[("error", err.to_string())],
                );
            }
        }
        scheduled.spawn(async move {
            tokio::select! {
                () = tokio::time::sleep(delay) => {
                    if !engine.state.stop.load(Ordering::SeqCst) {
                        // Equal-priority FIFO starts when the delay expires,
                        // matching the previous delayed-admission behavior.
                        queued.sequence = next_queue_sequence(engine.state.as_ref());
                        let _ = engine.enqueue_immediate(queued).await;
                    }
                }
                _ = stop_rx.changed() => {}
            }
            // Dropping a cancelled scheduled request releases both counters.
        });
        Ok(())
    }

    async fn enqueue_item(&self, item: Item) -> SilkwormResult<()> {
        let queued = QueuedItem {
            item,
            pending_guard: PendingWorkGuard::<true>::new(
                &self.state.item_pending,
                &self.state.item_pending_notify,
            ),
        };
        if let Err(err) = self.state.item_tx.send(queued).await {
            return Err(SilkwormError::Pipeline(format!(
                "Failed to enqueue item for pipelines: {err}"
            )));
        }
        Ok(())
    }

    async fn worker(self) {
        loop {
            let Some(queued) = self.next_ready_request().await else {
                break;
            };
            let QueuedRequest {
                request,
                pending_guard,
                ..
            } = queued;
            let result = self.process_request(request).await;
            if let Err(err) = result {
                self.state.stats.errors.fetch_add(1, Ordering::SeqCst);
                let error_text = err.to_string();
                self.state
                    .logger
                    .error("Failed to process request", &[("error", error_text)]);
                if self.state.fail_fast {
                    record_fatal_error_state(self.state.as_ref(), err).await;
                    drop(pending_guard);
                    self.shutdown();
                    break;
                }
            }

            drop(pending_guard);

            if self.state.stop.load(Ordering::SeqCst) {
                break;
            }
        }
    }

    async fn spawn_item_worker(&self) -> SilkwormResult<tokio::task::JoinHandle<()>> {
        let mut rx_slot = self.state.item_rx.lock().await;
        let Some(mut receiver) = rx_slot.take() else {
            return Err(SilkwormError::Spider(
                "item worker already started".to_string(),
            ));
        };

        let state = self.state.clone();
        Ok(tokio::spawn(async move {
            let mut stop_rx = state.stop_rx.clone();
            loop {
                if state.stop.load(Ordering::SeqCst) {
                    while receiver.try_recv().is_ok() {}
                    break;
                }
                tokio::select! {
                    maybe_item = receiver.recv() => {
                        let Some(QueuedItem { item, pending_guard }) = maybe_item else { break };
                        let mut current = item;
                        let mut stop_after_item = false;
                        for pipe in &state.item_pipelines {
                            match pipe.process_item(current, state.spider.clone()).await {
                                Ok(next) => current = next,
                                Err(err) => {
                                    state.stats.errors.fetch_add(1, Ordering::SeqCst);
                                    let error_text = err.to_string();
                                    state.logger.error(
                                        "Failed to process item",
                                        &[("error", error_text)],
                                    );
                                    if state.fail_fast {
                                        record_fatal_error_state(state.as_ref(), err).await;
                                        stop_after_item = true;
                                    }
                                    break;
                                }
                            }
                        }
                        drop(pending_guard);
                        if stop_after_item {
                            signal_stop_state(state.as_ref());
                            break;
                        }
                    }
                    _ = stop_rx.changed() => {
                        while receiver.try_recv().is_ok() {}
                        break;
                    }
                }
            }
        }))
    }

    async fn next_ready_request(&self) -> Option<QueuedRequest<S>> {
        let mut stop_rx = self.state.stop_rx.clone();
        loop {
            let notified = self.state.ready_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.state.stop.load(Ordering::SeqCst) {
                return None;
            }

            if let Some(mut queued) = {
                let mut ready = self.state.ready_queue.lock().await;
                ready.pop()
            } {
                drop(queued.queue_slot.take());
                return Some(queued);
            }

            tokio::select! {
                () = notified => {}
                _ = stop_rx.changed() => {
                    if *stop_rx.borrow() {
                        return None;
                    }
                }
            }
        }
    }

    async fn process_request(&self, req: Request<S>) -> SilkwormResult<()> {
        let mut req = req;
        for mw in &self.state.request_middlewares {
            req = mw.process_request(req, self.state.spider.clone()).await;
        }

        if let Some(delay) = take_request_delay(&mut req) {
            self.enqueue_delayed(req, delay).await?;
            return Ok(());
        }

        self.state
            .stats
            .requests_sent
            .fetch_add(1, Ordering::SeqCst);

        let resp = self.state.http.fetch(req).await?;
        self.state
            .stats
            .responses_received
            .fetch_add(1, Ordering::SeqCst);
        self.handle_response(resp).await
    }

    async fn handle_response(&self, response: Response<S>) -> SilkwormResult<()> {
        let mut processed = ResponseAction::Response(response);
        for mw in &self.state.response_middlewares {
            match processed {
                ResponseAction::Response(resp) => {
                    processed = mw.process_response(resp, self.state.spider.clone()).await;
                }
                ResponseAction::Request(_) => break,
            }
        }

        match processed {
            ResponseAction::Request(req) => {
                self.enqueue(req).await?;
                Ok(())
            }
            ResponseAction::Response(resp) => {
                let callback = resp.request.callback.clone();
                let outputs = if let Some(cb) = callback {
                    cb(self.state.spider.clone(), resp).await
                } else {
                    let html = resp.into_html(self.state.html_max_size_bytes);
                    self.state.spider.parse(html).await
                };
                for output in outputs? {
                    match output {
                        SpiderOutput::Request(req) => {
                            self.enqueue(*req).await?;
                        }
                        SpiderOutput::Item(item) => {
                            self.state
                                .stats
                                .items_scraped
                                .fetch_add(1, Ordering::SeqCst);
                            self.enqueue_item(item).await?;
                        }
                    }
                }
                Ok(())
            }
        }
    }

    async fn await_idle_or_worker_health(
        &self,
        workers: &mut JoinSet<()>,
        item_worker: &tokio::task::JoinHandle<()>,
    ) -> SilkwormResult<()> {
        loop {
            let pending_changed = self.state.pending_notify.notified();
            let items_changed = self.state.item_pending_notify.notified();
            let fatal_error = self.state.fatal_error_notify.notified();
            tokio::pin!(pending_changed, items_changed, fatal_error);
            pending_changed.as_mut().enable();
            items_changed.as_mut().enable();
            fatal_error.as_mut().enable();
            self.reap_scheduled_requests().await;
            if let Some(err) = self.take_fatal_error().await {
                return Err(err);
            }
            if self.state.pending.load(Ordering::SeqCst) == 0
                && self.state.item_pending.load(Ordering::SeqCst) == 0
            {
                return Ok(());
            }
            if item_worker.is_finished() {
                return Err(SilkwormError::Spider(
                    "item worker exited unexpectedly".to_string(),
                ));
            }

            tokio::select! {
                () = pending_changed => {}
                () = items_changed => {}
                () = fatal_error => {
                    if let Some(err) = self.take_fatal_error().await {
                        return Err(err);
                    }
                }
                worker = workers.join_next() => {
                    if let Some(err) = self.take_fatal_error().await {
                        return Err(err);
                    }
                    return match worker {
                        Some(Ok(())) => Err(SilkwormError::Spider(
                            "worker task exited unexpectedly".to_string(),
                        )),
                        Some(Err(err)) => Err(SilkwormError::Spider(format!("worker task failed: {err}"))),
                        None => Err(SilkwormError::Spider(
                            "all worker tasks exited unexpectedly".to_string(),
                        )),
                    };
                }
            }
        }
    }

    async fn join_workers(&self, workers: &mut JoinSet<()>) -> Option<SilkwormError> {
        let mut first_error = None;
        while let Some(res) = workers.join_next().await {
            if let Err(err) = res {
                self.state
                    .logger
                    .error("Worker task failed", &[("error", format!("{err}"))]);
                if first_error.is_none() {
                    first_error = Some(SilkwormError::Spider(format!("worker task failed: {err}")));
                }
            }
        }
        first_error
    }

    async fn join_item_worker(
        &self,
        item_worker: &mut tokio::task::JoinHandle<()>,
    ) -> Option<SilkwormError> {
        match item_worker.await {
            Ok(()) => None,
            Err(err) => {
                self.state
                    .logger
                    .error("Item worker failed", &[("error", format!("{err}"))]);
                Some(SilkwormError::Spider(format!("item worker failed: {err}")))
            }
        }
    }

    async fn take_fatal_error(&self) -> Option<SilkwormError> {
        let mut slot = self.state.fatal_error.lock().await;
        slot.take()
    }

    async fn reap_scheduled_requests(&self) {
        let mut scheduled = self.state.scheduled_requests.lock().await;
        while let Some(res) = scheduled.try_join_next() {
            if let Err(err) = res {
                self.state.logger.error(
                    "Scheduled request task failed",
                    &[("error", format!("{err}"))],
                );
            }
        }
    }

    async fn shutdown_scheduled_requests(&self) {
        let mut scheduled = {
            let mut guard = self.state.scheduled_requests.lock().await;
            let mut scheduled = JoinSet::new();
            std::mem::swap(&mut *guard, &mut scheduled);
            scheduled
        };
        scheduled.shutdown().await;
        let mut guard = self.state.scheduled_requests.lock().await;
        *guard = scheduled;
    }

    fn shutdown(&self) {
        signal_stop_state(self.state.as_ref());
    }

    async fn log_statistics(self, interval: Duration) {
        let mut ticker = tokio::time::interval(interval);
        let mut stop_rx = self.state.stop_rx.clone();
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if self.state.stop.load(Ordering::SeqCst) {
                        break;
                    }
                    self.state
                        .logger
                        .info("Crawl statistics", &self.stats_payload());
                }
                _ = stop_rx.changed() => {
                    if *stop_rx.borrow() {
                        break;
                    }
                }
            }
        }
    }

    fn stats_payload(&self) -> Vec<(&str, String)> {
        let elapsed_duration = self.state.stats.elapsed();
        let elapsed = elapsed_duration.as_secs_f64();
        let requests_sent = self.state.stats.requests_sent.load(Ordering::SeqCst);
        let responses_received = self.state.stats.responses_received.load(Ordering::SeqCst);
        let items_scraped = self.state.stats.items_scraped.load(Ordering::SeqCst);
        let errors = self.state.stats.errors.load(Ordering::SeqCst);
        let pending = self.state.pending.load(Ordering::SeqCst);
        let pending_items = self.state.item_pending.load(Ordering::SeqCst);
        let seen = self.state.seen_count.load(Ordering::SeqCst);
        let rate = format_requests_per_second(requests_sent, elapsed_duration);

        let mut out = vec![
            ("elapsed_seconds", format!("{elapsed:.1}")),
            ("requests_sent", requests_sent.to_string()),
            ("responses_received", responses_received.to_string()),
            ("items_scraped", items_scraped.to_string()),
            ("errors", errors.to_string()),
            ("pending_requests", pending.to_string()),
            ("pending_items", pending_items.to_string()),
            ("requests_per_second", rate),
            ("seen_requests", seen.to_string()),
        ];

        if let Some((rss_bytes, vms_bytes)) = memory_usage_bytes() {
            out.push(("memory_rss_mb", format_mebibytes(rss_bytes)));
            out.push(("memory_vms_mb", format_mebibytes(vms_bytes)));
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::{Engine, EngineConfig, QueuedRequest, SeenRequests, Stats, request_fingerprint};
    use crate::errors::SilkwormError;
    use crate::middlewares::{MiddlewareFuture, RequestMiddleware};
    use crate::pipelines::{ItemPipeline, PipelineFuture};
    use crate::request::{Request, SpiderResult};
    use crate::response::HtmlResponse;
    use crate::spider::Spider;
    use crate::types::Item;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    struct TestSpider;

    impl Spider for TestSpider {
        fn name(&self) -> &str {
            "test"
        }

        async fn parse(&self, _response: HtmlResponse<Self>) -> SpiderResult<Self> {
            Ok(Vec::new())
        }
    }

    struct StartSpider;

    impl Spider for StartSpider {
        fn name(&self) -> &str {
            "start"
        }

        fn start_urls(&self) -> Vec<&str> {
            vec!["https://example.com"]
        }

        async fn parse(&self, _response: HtmlResponse<Self>) -> SpiderResult<Self> {
            Ok(Vec::new())
        }
    }

    struct InvalidUrlSpider;

    impl Spider for InvalidUrlSpider {
        fn name(&self) -> &str {
            "invalid-url"
        }

        fn start_urls(&self) -> Vec<&str> {
            vec!["http://[::1"]
        }

        async fn parse(&self, _response: HtmlResponse<Self>) -> SpiderResult<Self> {
            Ok(Vec::new())
        }
    }

    struct CountingSpider {
        close_calls: Arc<AtomicUsize>,
    }

    impl Spider for CountingSpider {
        fn name(&self) -> &str {
            "counting"
        }

        async fn parse(&self, _response: HtmlResponse<Self>) -> SpiderResult<Self> {
            Ok(Vec::new())
        }

        async fn close(&self) {
            self.close_calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct PanicRequestMiddleware;

    impl<S: Spider> RequestMiddleware<S> for PanicRequestMiddleware {
        fn process_request<'a>(
            &'a self,
            _request: Request<S>,
            _spider: Arc<S>,
        ) -> MiddlewareFuture<'a, Request<S>> {
            Box::pin(async move { panic!("middleware panic") })
        }
    }

    struct TestPipeline {
        close_calls: Arc<AtomicUsize>,
        fail_close: bool,
    }

    impl TestPipeline {
        fn new(close_calls: Arc<AtomicUsize>, fail_close: bool) -> Self {
            TestPipeline {
                close_calls,
                fail_close,
            }
        }
    }

    impl<S: Spider> ItemPipeline<S> for TestPipeline {
        fn open<'a>(&'a self, _spider: Arc<S>) -> PipelineFuture<'a, crate::SilkwormResult<()>> {
            Box::pin(async move { Ok(()) })
        }

        fn close<'a>(&'a self, _spider: Arc<S>) -> PipelineFuture<'a, crate::SilkwormResult<()>> {
            Box::pin(async move {
                self.close_calls.fetch_add(1, Ordering::SeqCst);
                if self.fail_close {
                    return Err(SilkwormError::Pipeline("pipeline close failed".to_string()));
                }
                Ok(())
            })
        }

        fn process_item<'a>(
            &'a self,
            item: Item,
            _spider: Arc<S>,
        ) -> PipelineFuture<'a, crate::SilkwormResult<Item>> {
            Box::pin(async move { Ok(item) })
        }
    }

    #[test]
    fn stats_elapsed_defaults_to_zero() {
        let stats = Stats::new();
        assert_eq!(stats.elapsed(), std::time::Duration::ZERO);
    }

    #[test]
    fn engine_new_rejects_zero_concurrency() {
        let config = EngineConfig::<TestSpider> {
            concurrency: 0,
            request_middlewares: Vec::new(),
            response_middlewares: Vec::new(),
            item_pipelines: Vec::new(),
            request_timeout: None,
            log_stats_interval: None,
            max_pending_requests: None,
            max_seen_requests: None,
            html_max_size_bytes: 1,
            keep_alive: false,
            fail_fast: false,
        };
        let result = Engine::new(TestSpider, config);
        match result {
            Err(SilkwormError::Config(_)) => {}
            Err(other) => panic!("expected config error, got {other:?}"),
            Ok(_) => panic!("expected error, got ok"),
        }
    }

    #[test]
    fn engine_new_rejects_zero_max_seen_requests() {
        let config = EngineConfig::<TestSpider> {
            concurrency: 1,
            request_middlewares: Vec::new(),
            response_middlewares: Vec::new(),
            item_pipelines: Vec::new(),
            request_timeout: None,
            log_stats_interval: None,
            max_pending_requests: None,
            max_seen_requests: Some(0),
            html_max_size_bytes: 1,
            keep_alive: false,
            fail_fast: false,
        };
        let result = Engine::new(TestSpider, config);
        match result {
            Err(SilkwormError::Config(_)) => {}
            Err(other) => panic!("expected config error, got {other:?}"),
            Ok(_) => panic!("expected error, got ok"),
        }
    }

    #[test]
    fn engine_new_rejects_zero_max_pending_requests() {
        let config = EngineConfig::<TestSpider> {
            concurrency: 1,
            request_middlewares: Vec::new(),
            response_middlewares: Vec::new(),
            item_pipelines: Vec::new(),
            request_timeout: None,
            log_stats_interval: None,
            max_pending_requests: Some(0),
            max_seen_requests: None,
            html_max_size_bytes: 1,
            keep_alive: false,
            fail_fast: false,
        };
        let result = Engine::new(TestSpider, config);
        match result {
            Err(SilkwormError::Config(_)) => {}
            Err(other) => panic!("expected config error, got {other:?}"),
            Ok(_) => panic!("expected error, got ok"),
        }
    }

    #[test]
    fn seen_requests_with_limit_evicts_oldest() {
        let mut seen = SeenRequests::new(Some(2));
        assert!(seen.insert_if_new("https://a"));
        assert!(seen.insert_if_new("https://b"));
        assert!(!seen.insert_if_new("https://a"));
        assert!(seen.insert_if_new("https://c"));
        assert!(seen.insert_if_new("https://a"));
    }

    #[test]
    fn queued_requests_honor_priority_then_fifo() {
        let mut heap = std::collections::BinaryHeap::new();
        heap.push(QueuedRequest::new(
            Request::<TestSpider>::new("https://example.com/low").with_priority(1),
            0,
        ));
        heap.push(QueuedRequest::new(
            Request::<TestSpider>::new("https://example.com/high-old").with_priority(3),
            1,
        ));
        heap.push(QueuedRequest::new(
            Request::<TestSpider>::new("https://example.com/high-new").with_priority(3),
            2,
        ));

        assert_eq!(
            heap.pop().map(|queued| queued.request.url),
            Some("https://example.com/high-old".to_string())
        );
        assert_eq!(
            heap.pop().map(|queued| queued.request.url),
            Some("https://example.com/high-new".to_string())
        );
        assert_eq!(
            heap.pop().map(|queued| queued.request.url),
            Some("https://example.com/low".to_string())
        );
    }

    fn bounded_engine(limit: usize) -> Engine<TestSpider> {
        let config = crate::runner::RunConfig::<TestSpider>::new()
            .with_concurrency(1)
            .with_max_pending_requests(limit);
        Engine::new(TestSpider, config.into()).unwrap()
    }

    #[tokio::test]
    async fn capacity_covers_ready_and_delayed_requests_and_excludes_active_work() {
        let engine = bounded_engine(2);
        engine
            .enqueue(Request::new("https://example.com/ready"))
            .await
            .unwrap();
        engine
            .enqueue_delayed(
                Request::new("https://example.com/delayed"),
                Duration::from_secs(3600),
            )
            .await
            .unwrap();
        let budget = engine.state.queue_budget.as_ref().unwrap();
        assert_eq!(budget.waiting.load(Ordering::SeqCst), 2);
        assert_eq!(engine.state.pending.load(Ordering::SeqCst), 2);
        let active = engine.next_ready_request().await.unwrap();
        assert_eq!(budget.waiting.load(Ordering::SeqCst), 1);
        assert_eq!(engine.state.pending.load(Ordering::SeqCst), 2);
        engine
            .enqueue(Request::new("https://example.com/next"))
            .await
            .unwrap();
        assert_eq!(budget.waiting.load(Ordering::SeqCst), 2);
        drop(active);
        assert_eq!(engine.state.pending.load(Ordering::SeqCst), 2);
        engine.shutdown();
        engine.shutdown_scheduled_requests().await;
        engine.state.ready_queue.lock().await.clear();
        assert_eq!(budget.waiting.load(Ordering::SeqCst), 0);
        assert_eq!(engine.state.pending.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn full_queue_deduplicates_existing_requests_but_reports_new_overload() {
        let engine = bounded_engine(1);
        engine
            .enqueue(Request::new("https://example.com/first"))
            .await
            .unwrap();
        engine
            .enqueue(Request::new("https://example.com/first"))
            .await
            .unwrap();
        assert!(!engine.state.stop.load(Ordering::SeqCst));
        assert_eq!(engine.state.pending.load(Ordering::SeqCst), 1);
        let err = engine
            .enqueue(Request::new("https://example.com/overflow"))
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("max_pending_requests capacity (1) exceeded")
        );
        assert!(engine.state.stop.load(Ordering::SeqCst));
        assert!(engine.take_fatal_error().await.is_some());
        assert_eq!(engine.state.seen_count.load(Ordering::SeqCst), 1);
        assert!(
            !engine
                .state
                .seen
                .lock()
                .await
                .contains("GET https://example.com/overflow")
        );
        engine.state.ready_queue.lock().await.clear();
        assert_eq!(engine.state.pending.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cancelled_admission_releases_pending_and_capacity() {
        use std::future::{Future, poll_fn};
        use std::task::Poll;
        let engine = bounded_engine(1);
        let ready = engine.state.ready_queue.lock().await;
        let mut enqueue = Box::pin(engine.enqueue(Request::new("https://example.com/cancelled")));
        assert!(poll_fn(|cx| Poll::Ready(enqueue.as_mut().poll(cx).is_pending())).await);
        assert_eq!(engine.state.pending.load(Ordering::SeqCst), 1);
        assert_eq!(
            engine
                .state
                .queue_budget
                .as_ref()
                .unwrap()
                .waiting
                .load(Ordering::SeqCst),
            1
        );
        drop(enqueue);
        drop(ready);
        assert_eq!(engine.state.pending.load(Ordering::SeqCst), 0);
        assert_eq!(
            engine
                .state
                .queue_budget
                .as_ref()
                .unwrap()
                .waiting
                .load(Ordering::SeqCst),
            0
        );

        let scheduled = engine.state.scheduled_requests.lock().await;
        let mut enqueue = Box::pin(engine.enqueue_delayed(
            Request::new("https://example.com/delayed"),
            Duration::from_secs(3600),
        ));
        assert!(poll_fn(|cx| Poll::Ready(enqueue.as_mut().poll(cx).is_pending())).await);
        assert_eq!(engine.state.pending.load(Ordering::SeqCst), 1);
        drop(enqueue);
        drop(scheduled);
        assert_eq!(engine.state.pending.load(Ordering::SeqCst), 0);
        assert_eq!(
            engine
                .state
                .queue_budget
                .as_ref()
                .unwrap()
                .waiting
                .load(Ordering::SeqCst),
            0
        );
    }

    #[tokio::test]
    async fn real_ready_queue_preserves_priority_fifo_and_releases_all_slots() {
        let engine = bounded_engine(3);
        for (url, priority) in [("low", 1), ("high-old", 3), ("high-new", 3)] {
            engine
                .enqueue(Request::new(format!("https://example.com/{url}")).with_priority(priority))
                .await
                .unwrap();
        }
        for url in ["high-old", "high-new", "low"] {
            let queued = engine.next_ready_request().await.unwrap();
            assert_eq!(queued.request.url, format!("https://example.com/{url}"));
            drop(queued);
        }
        assert_eq!(engine.state.pending.load(Ordering::SeqCst), 0);
        assert_eq!(
            engine
                .state
                .queue_budget
                .as_ref()
                .unwrap()
                .waiting
                .load(Ordering::SeqCst),
            0
        );
    }

    #[tokio::test]
    async fn queued_request_accounting_does_not_keep_engine_alive() {
        let engine = bounded_engine(1);
        engine
            .enqueue(Request::new("https://example.com/queued"))
            .await
            .unwrap();
        let weak = Arc::downgrade(&engine.state);
        drop(engine);
        assert!(weak.upgrade().is_none());
    }

    #[tokio::test]
    async fn startup_overload_returns_error_and_releases_delayed_requests() {
        struct BurstStartSpider;
        impl Spider for BurstStartSpider {
            fn name(&self) -> &str {
                "burst-start"
            }
            async fn start_requests(&self) -> Vec<Request<Self>> {
                (0..10)
                    .map(|id| {
                        let mut req = Request::new(format!("https://example.com/{id}"));
                        req.set_retry_delay_secs(3600.0);
                        req
                    })
                    .collect()
            }
            async fn parse(&self, _response: HtmlResponse<Self>) -> SpiderResult<Self> {
                Ok(Vec::new())
            }
        }
        let config = crate::runner::RunConfig::<BurstStartSpider>::new()
            .with_concurrency(2)
            .with_max_pending_requests(2)
            .with_fail_fast(false);
        let engine = Engine::new(BurstStartSpider, config.into()).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), engine.run())
            .await
            .expect("overload must not deadlock");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("max_pending_requests capacity")
        );
        assert_eq!(engine.state.pending.load(Ordering::SeqCst), 0);
        assert_eq!(
            engine
                .state
                .queue_budget
                .as_ref()
                .unwrap()
                .waiting
                .load(Ordering::SeqCst),
            0
        );
    }

    #[tokio::test]
    async fn cancelled_item_admission_releases_its_pending_count() {
        use std::future::{Future, poll_fn};
        use std::task::Poll;
        let engine = bounded_engine(1);
        engine.enqueue_item(Item::from(1)).await.unwrap();
        let mut enqueue = Box::pin(engine.enqueue_item(Item::from(2)));
        assert!(poll_fn(|cx| Poll::Ready(enqueue.as_mut().poll(cx).is_pending())).await);
        assert_eq!(engine.state.item_pending.load(Ordering::SeqCst), 2);
        drop(enqueue);
        assert_eq!(engine.state.item_pending.load(Ordering::SeqCst), 1);
        let mut receiver = engine.state.item_rx.lock().await.take().unwrap();
        drop(receiver.recv().await.unwrap());
        assert_eq!(engine.state.item_pending.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn failing_item_worker_releases_buffered_items() {
        struct FailingPipeline;
        impl ItemPipeline<TestSpider> for FailingPipeline {
            fn open(
                &self,
                _spider: Arc<TestSpider>,
            ) -> PipelineFuture<'_, crate::SilkwormResult<()>> {
                Box::pin(async { Ok(()) })
            }
            fn close(
                &self,
                _spider: Arc<TestSpider>,
            ) -> PipelineFuture<'_, crate::SilkwormResult<()>> {
                Box::pin(async { Ok(()) })
            }
            fn process_item(
                &self,
                _item: Item,
                _spider: Arc<TestSpider>,
            ) -> PipelineFuture<'_, crate::SilkwormResult<Item>> {
                Box::pin(async { Err(SilkwormError::Pipeline("write failed".to_string())) })
            }
        }
        let config = crate::runner::RunConfig::<TestSpider>::new()
            .with_concurrency(1)
            .with_max_pending_requests(4)
            .with_fail_fast(true)
            .with_item_pipeline(FailingPipeline);
        let engine = Engine::new(TestSpider, config.into()).unwrap();
        for id in 0..3 {
            engine.enqueue_item(Item::from(id)).await.unwrap();
        }
        let task = engine.spawn_item_worker().await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(engine.state.item_pending.load(Ordering::SeqCst), 0);
        assert!(engine.take_fatal_error().await.is_some());
    }

    #[tokio::test]
    async fn callback_fanout_overload_stops_crawl_even_without_fail_fast() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        struct FanoutSpider {
            url: String,
        }
        impl Spider for FanoutSpider {
            fn name(&self) -> &str {
                "fanout"
            }
            fn start_urls(&self) -> Vec<&str> {
                vec![&self.url]
            }
            async fn parse(&self, _response: HtmlResponse<Self>) -> SpiderResult<Self> {
                Ok((0..5)
                    .map(|id| {
                        let mut req = Request::new(format!("{}/child/{id}", self.url));
                        req.set_retry_delay_secs(3600.0);
                        crate::request::SpiderOutput::Request(Box::new(req))
                    })
                    .collect())
            }
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 1024];
            let mut received = Vec::new();
            loop {
                let read = socket.read(&mut request).await.unwrap();
                assert!(
                    read > 0,
                    "connection closed before complete request headers"
                );
                received.extend_from_slice(&request[..read]);
                if received.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        let config = crate::runner::RunConfig::<FanoutSpider>::new()
            .with_concurrency(2)
            .with_max_pending_requests(2)
            .with_fail_fast(false);
        let engine = Engine::new(FanoutSpider { url }, config.into()).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), engine.run())
            .await
            .expect("fan-out must not deadlock");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("max_pending_requests capacity (2) exceeded")
        );
        assert_eq!(engine.state.pending.load(Ordering::SeqCst), 0);
        assert_eq!(engine.state.item_pending.load(Ordering::SeqCst), 0);
        assert_eq!(
            engine
                .state
                .queue_budget
                .as_ref()
                .unwrap()
                .waiting
                .load(Ordering::SeqCst),
            0
        );
        assert!(engine.state.ready_queue.lock().await.is_empty());
        assert!(engine.state.scheduled_requests.lock().await.is_empty());
        server.await.unwrap();
    }

    #[test]
    fn request_fingerprint_distinguishes_params_and_method() {
        let get_a = Request::<()>::new("https://example.com/search")
            .with_param("q", "rust")
            .with_param("page", "1");
        let get_b = Request::<()>::new("https://example.com/search")
            .with_param("q", "rust")
            .with_param("page", "2");
        let post = Request::<()>::new("https://example.com/search").with_method("POST");

        assert_ne!(request_fingerprint(&get_a), request_fingerprint(&get_b));
        assert_ne!(request_fingerprint(&get_a), request_fingerprint(&post));
    }

    #[test]
    fn request_fingerprint_keeps_duplicate_query_values() {
        let req_a = Request::<()>::new("https://example.com/search?tag=b&tag=a&x=1");
        let req_b = Request::<()>::new("https://example.com/search?x=1&tag=a&tag=b");
        let req_c = Request::<()>::new("https://example.com/search?tag=a&x=1");

        assert_eq!(request_fingerprint(&req_a), request_fingerprint(&req_b));
        assert_ne!(request_fingerprint(&req_a), request_fingerprint(&req_c));
    }

    #[test]
    fn request_fingerprint_ignores_fragment() {
        let req_a = Request::<()>::new("https://example.com/search?q=rust#top");
        let req_b = Request::<()>::new("https://example.com/search?q=rust#bottom");

        assert_eq!(request_fingerprint(&req_a), request_fingerprint(&req_b));
    }

    #[tokio::test]
    async fn run_with_fail_fast_disabled_continues_after_request_error() {
        let config = EngineConfig::<InvalidUrlSpider> {
            concurrency: 1,
            request_middlewares: Vec::new(),
            response_middlewares: Vec::new(),
            item_pipelines: Vec::new(),
            request_timeout: None,
            log_stats_interval: None,
            max_pending_requests: None,
            max_seen_requests: None,
            html_max_size_bytes: 1,
            keep_alive: false,
            fail_fast: false,
        };
        let engine = Engine::new(InvalidUrlSpider, config).expect("engine");

        let run_result = tokio::time::timeout(Duration::from_secs(1), engine.run()).await;
        assert!(run_result.is_ok(), "engine.run timed out");
        assert!(run_result.expect("timeout result").is_ok());
    }

    #[tokio::test]
    async fn run_with_fail_fast_enabled_returns_first_request_error() {
        let config = EngineConfig::<InvalidUrlSpider> {
            concurrency: 1,
            request_middlewares: Vec::new(),
            response_middlewares: Vec::new(),
            item_pipelines: Vec::new(),
            request_timeout: None,
            log_stats_interval: None,
            max_pending_requests: None,
            max_seen_requests: None,
            html_max_size_bytes: 1,
            keep_alive: false,
            fail_fast: true,
        };
        let engine = Engine::new(InvalidUrlSpider, config).expect("engine");

        let run_result = tokio::time::timeout(Duration::from_secs(1), engine.run()).await;
        assert!(run_result.is_ok(), "engine.run timed out");
        match run_result.expect("timeout result") {
            Err(SilkwormError::Http(message)) => {
                assert!(message.contains("Invalid URL"));
            }
            Err(other) => panic!("expected http error, got {other:?}"),
            Ok(_) => panic!("expected error, got ok"),
        }
    }

    #[tokio::test]
    async fn run_returns_error_when_worker_panics() {
        let config = EngineConfig::<StartSpider> {
            concurrency: 1,
            request_middlewares: vec![Arc::new(PanicRequestMiddleware)],
            response_middlewares: Vec::new(),
            item_pipelines: Vec::new(),
            request_timeout: None,
            log_stats_interval: None,
            max_pending_requests: None,
            max_seen_requests: None,
            html_max_size_bytes: 1,
            keep_alive: false,
            fail_fast: false,
        };
        let engine = Engine::new(StartSpider, config).expect("engine");

        let run_result = tokio::time::timeout(Duration::from_secs(1), engine.run()).await;
        assert!(run_result.is_ok(), "engine.run timed out");
        match run_result.expect("timeout result") {
            Err(SilkwormError::Spider(message)) => {
                assert!(message.contains("worker task"));
            }
            Err(other) => panic!("expected spider error, got {other:?}"),
            Ok(_) => panic!("expected error, got ok"),
        }
    }

    #[tokio::test]
    async fn close_spider_closes_all_pipelines_even_on_error() {
        let close_calls = Arc::new(AtomicUsize::new(0));
        let pipeline_a_calls = Arc::new(AtomicUsize::new(0));
        let pipeline_b_calls = Arc::new(AtomicUsize::new(0));

        let spider = CountingSpider {
            close_calls: close_calls.clone(),
        };
        let config = EngineConfig::<CountingSpider> {
            concurrency: 1,
            request_middlewares: Vec::new(),
            response_middlewares: Vec::new(),
            item_pipelines: vec![
                Arc::new(TestPipeline::new(pipeline_a_calls.clone(), true)),
                Arc::new(TestPipeline::new(pipeline_b_calls.clone(), false)),
            ],
            request_timeout: None,
            log_stats_interval: None,
            max_pending_requests: None,
            max_seen_requests: None,
            html_max_size_bytes: 1,
            keep_alive: false,
            fail_fast: false,
        };

        let engine = Engine::new(spider, config).expect("engine");
        let result = engine.run().await;

        match result {
            Err(SilkwormError::Pipeline(_)) => {}
            Err(other) => panic!("expected pipeline error, got {other:?}"),
            Ok(_) => panic!("expected error, got ok"),
        }

        assert_eq!(pipeline_a_calls.load(Ordering::SeqCst), 1);
        assert_eq!(pipeline_b_calls.load(Ordering::SeqCst), 1);
        assert_eq!(close_calls.load(Ordering::SeqCst), 1);
    }
}
