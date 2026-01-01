use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, Mutex as AsyncMutex, Notify};
use tokio::task::JoinSet;

use crate::errors::{SilkwormError, SilkwormResult};
use crate::http::HttpClient;
use crate::logging::{complete_logs, get_logger, Logger};
use crate::middlewares::{RequestMiddleware, ResponseAction, ResponseMiddleware};
use crate::pipelines::ItemPipeline;
use crate::request::{Request, SpiderOutput, SpiderResult};
use crate::response::Response;
use crate::spider::Spider;
use crate::types::Item;

struct Stats {
    start_time: Mutex<Option<Instant>>,
    requests_sent: AtomicUsize,
    responses_received: AtomicUsize,
    items_scraped: AtomicUsize,
    errors: AtomicUsize,
}

impl Stats {
    fn new() -> Self {
        Stats {
            start_time: Mutex::new(None),
            requests_sent: AtomicUsize::new(0),
            responses_received: AtomicUsize::new(0),
            items_scraped: AtomicUsize::new(0),
            errors: AtomicUsize::new(0),
        }
    }

    fn set_start_time(&self, when: Instant) {
        let mut guard = self.start_time.lock().expect("stats lock");
        *guard = Some(when);
    }

    fn elapsed(&self) -> Duration {
        let guard = self.start_time.lock().expect("stats lock");
        guard.map(|start| start.elapsed()).unwrap_or_default()
    }
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
    pub max_pending_requests: Option<usize>,
    pub html_max_size_bytes: usize,
    pub keep_alive: bool,
}

struct EngineState<S: Spider> {
    spider: Arc<S>,
    http: HttpClient,
    queue_tx: mpsc::Sender<Request<S>>,
    queue_rx: AsyncMutex<mpsc::Receiver<Request<S>>>,
    seen: AsyncMutex<HashSet<String>>,
    seen_count: AtomicUsize,
    stop: AtomicBool,
    pending: AtomicUsize,
    pending_notify: Notify,
    stop_notify: Notify,
    request_middlewares: Vec<Arc<dyn RequestMiddleware<S>>>,
    response_middlewares: Vec<Arc<dyn ResponseMiddleware<S>>>,
    item_pipelines: Vec<Arc<dyn ItemPipeline<S>>>,
    log_stats_interval: Option<Duration>,
    stats: Stats,
    logger: Logger,
    html_max_size_bytes: usize,
}

impl<S: Spider> Engine<S> {
    pub fn new(spider: S, config: EngineConfig<S>) -> SilkwormResult<Self> {
        let EngineConfig {
            concurrency,
            request_middlewares,
            response_middlewares,
            item_pipelines,
            request_timeout,
            log_stats_interval,
            max_pending_requests,
            html_max_size_bytes,
            keep_alive,
        } = config;
        let queue_size = max_pending_requests.unwrap_or(concurrency * 10).max(1);
        let (queue_tx, queue_rx) = mpsc::channel(queue_size);
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
            queue_tx,
            queue_rx: AsyncMutex::new(queue_rx),
            seen: AsyncMutex::new(HashSet::new()),
            seen_count: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
            pending: AtomicUsize::new(0),
            pending_notify: Notify::new(),
            stop_notify: Notify::new(),
            request_middlewares,
            response_middlewares,
            item_pipelines,
            log_stats_interval,
            stats: Stats::new(),
            logger,
            html_max_size_bytes,
        };
        Ok(Engine {
            state: Arc::new(state),
        })
    }

    pub async fn run(&self) -> SilkwormResult<()> {
        self.state.logger.info(
            "Starting engine",
            &[("spider", self.state.spider.name().to_string())],
        );
        self.state.stats.set_start_time(Instant::now());

        let mut join_set = JoinSet::new();
        for _ in 0..self.state.http.concurrency {
            let engine = Self {
                state: self.state.clone(),
            };
            join_set.spawn(async move {
                engine.worker().await;
            });
        }

        if let Some(interval) = self.state.log_stats_interval {
            if interval > Duration::from_secs(0) {
                let engine = Self {
                    state: self.state.clone(),
                };
                join_set.spawn(async move {
                    engine.log_statistics(interval).await;
                });
            }
        }

        if let Err(err) = self.open_spider().await {
            self.state
                .logger
                .error("Failed to open spider", &[("error", err.to_string())]);
            self.shutdown().await;
            return Err(err);
        }

        self.await_idle().await;
        self.shutdown().await;

        while let Some(res) = join_set.join_next().await {
            if let Err(err) = res {
                self.state
                    .logger
                    .error("Worker task failed", &[("error", format!("{err}"))]);
            }
        }

        self.state
            .logger
            .info("Final crawl statistics", &self.stats_payload());

        self.state.http.close().await;
        self.close_spider().await?;
        complete_logs();
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
        for pipe in &self.state.item_pipelines {
            pipe.close(self.state.spider.clone()).await?;
        }
        self.state.spider.close().await;
        Ok(())
    }

    async fn enqueue(&self, req: Request<S>) -> SilkwormResult<()> {
        if !req.dont_filter {
            let mut seen = self.state.seen.lock().await;
            if seen.contains(&req.url) {
                self.state
                    .logger
                    .debug("Skipping already seen request", &[("url", req.url.clone())]);
                return Ok(());
            }
            seen.insert(req.url.clone());
            self.state.seen_count.fetch_add(1, Ordering::SeqCst);
        }

        self.state
            .logger
            .debug("Enqueued request", &[("url", req.url.clone())]);

        self.state.pending.fetch_add(1, Ordering::SeqCst);
        if let Err(err) = self.state.queue_tx.send(req).await {
            self.state.pending.fetch_sub(1, Ordering::SeqCst);
            return Err(SilkwormError::Http(format!(
                "Failed to enqueue request: {err}"
            )));
        }
        Ok(())
    }

    async fn worker(self) {
        loop {
            if self.state.stop.load(Ordering::SeqCst) {
                break;
            }

            let req_opt = tokio::select! {
                req = async {
                    let mut rx = self.state.queue_rx.lock().await;
                    rx.recv().await
                } => req,
                _ = self.state.stop_notify.notified() => None,
            };
            let Some(req) = req_opt else { break };

            let result = self.process_request(req).await;
            if let Err(err) = result {
                self.state.errors_increment();
                self.state
                    .logger
                    .error("Failed to process request", &[("error", err.to_string())]);
            }

            self.finish_request();

            if self.state.stop.load(Ordering::SeqCst) {
                break;
            }
        }
    }

    async fn process_request(&self, req: Request<S>) -> SilkwormResult<()> {
        let req = self.apply_request_middlewares(req).await;
        self.state.requests_sent_increment();

        let resp = self.state.http.fetch(req).await?;
        self.state.responses_received_increment();
        self.handle_response(resp).await
    }

    async fn apply_request_middlewares(&self, req: Request<S>) -> Request<S> {
        let mut current = req;
        for mw in &self.state.request_middlewares {
            current = mw.process_request(current, self.state.spider.clone()).await;
        }
        current
    }

    async fn handle_response(&self, response: Response<S>) -> SilkwormResult<()> {
        let processed = self.apply_response_middlewares(response).await?;
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
                self.handle_outputs(outputs).await
            }
        }
    }

    async fn apply_response_middlewares(
        &self,
        response: Response<S>,
    ) -> SilkwormResult<ResponseAction<S>> {
        let mut current = ResponseAction::Response(response);
        for mw in &self.state.response_middlewares {
            match current {
                ResponseAction::Response(resp) => {
                    current = mw.process_response(resp, self.state.spider.clone()).await;
                }
                ResponseAction::Request(_) => break,
            }
        }
        Ok(current)
    }

    async fn handle_outputs(&self, outputs: SpiderResult<S>) -> SilkwormResult<()> {
        for output in outputs {
            match output {
                SpiderOutput::Request(req) => {
                    self.enqueue(*req).await?;
                }
                SpiderOutput::Item(item) => {
                    self.process_item(item).await?;
                }
            }
        }
        Ok(())
    }

    async fn process_item(&self, item: Item) -> SilkwormResult<()> {
        self.state.items_scraped_increment();
        let mut current = item;
        for pipe in &self.state.item_pipelines {
            current = pipe
                .process_item(current, self.state.spider.clone())
                .await?;
        }
        Ok(())
    }

    async fn await_idle(&self) {
        loop {
            if self.state.pending.load(Ordering::SeqCst) == 0 {
                break;
            }
            let notified = self.state.pending_notify.notified();
            if self.state.pending.load(Ordering::SeqCst) == 0 {
                break;
            }
            notified.await;
        }
    }

    async fn shutdown(&self) {
        self.state.stop.store(true, Ordering::SeqCst);
        self.state.stop_notify.notify_waiters();
    }

    fn finish_request(&self) {
        let _ = self
            .state
            .pending
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                Some(value.saturating_sub(1))
            });
        self.state.pending_notify.notify_waiters();
    }

    async fn log_statistics(self, interval: Duration) {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            if self.state.stop.load(Ordering::SeqCst) {
                break;
            }
            self.state
                .logger
                .info("Crawl statistics", &self.stats_payload());
        }
    }

    fn stats_payload(&self) -> Vec<(&str, String)> {
        let elapsed = self.state.stats.elapsed().as_secs_f64();
        let requests_sent = self.state.stats.requests_sent.load(Ordering::SeqCst);
        let responses_received = self.state.stats.responses_received.load(Ordering::SeqCst);
        let items_scraped = self.state.stats.items_scraped.load(Ordering::SeqCst);
        let errors = self.state.stats.errors.load(Ordering::SeqCst);
        let pending = self.state.pending.load(Ordering::SeqCst);
        let seen = self.state.seen_count.load(Ordering::SeqCst);
        let rate = if elapsed > 0.0 {
            requests_sent as f64 / elapsed
        } else {
            0.0
        };

        vec![
            ("elapsed_seconds", format!("{:.1}", elapsed)),
            ("requests_sent", requests_sent.to_string()),
            ("responses_received", responses_received.to_string()),
            ("items_scraped", items_scraped.to_string()),
            ("errors", errors.to_string()),
            ("pending_requests", pending.to_string()),
            ("requests_per_second", format!("{:.2}", rate)),
            ("seen_requests", seen.to_string()),
        ]
    }
}

impl<S: Spider> EngineState<S> {
    fn requests_sent_increment(&self) {
        self.stats.requests_sent.fetch_add(1, Ordering::SeqCst);
    }

    fn responses_received_increment(&self) {
        self.stats.responses_received.fetch_add(1, Ordering::SeqCst);
    }

    fn items_scraped_increment(&self) {
        self.stats.items_scraped.fetch_add(1, Ordering::SeqCst);
    }

    fn errors_increment(&self) {
        self.stats.errors.fetch_add(1, Ordering::SeqCst);
    }
}
