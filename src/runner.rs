use std::sync::Arc;
use std::time::Duration;

use crate::engine::{Engine, EngineConfig};
use crate::errors::{SilkwormError, SilkwormResult};
use crate::middlewares::{RequestMiddleware, ResponseMiddleware};
use crate::pipelines::ItemPipeline;
use crate::spider::Spider;

const DEFAULT_MAX_SEEN_REQUESTS: usize = 100_000;

pub struct RunConfig<S: Spider> {
    pub concurrency: usize,
    pub request_middlewares: Vec<Arc<dyn RequestMiddleware<S>>>,
    pub response_middlewares: Vec<Arc<dyn ResponseMiddleware<S>>>,
    pub item_pipelines: Vec<Arc<dyn ItemPipeline<S>>>,
    pub request_timeout: Option<Duration>,
    pub log_stats_interval: Option<Duration>,
    pub max_pending_requests: Option<usize>,
    pub max_seen_requests: Option<usize>,
    pub html_max_size_bytes: usize,
    pub keep_alive: bool,
}

impl<S: Spider> Default for RunConfig<S> {
    fn default() -> Self {
        RunConfig {
            concurrency: 16,
            request_middlewares: Vec::new(),
            response_middlewares: Vec::new(),
            item_pipelines: Vec::new(),
            request_timeout: None,
            log_stats_interval: None,
            max_pending_requests: None,
            max_seen_requests: Some(DEFAULT_MAX_SEEN_REQUESTS),
            html_max_size_bytes: 5_000_000,
            keep_alive: false,
        }
    }
}

impl<S: Spider> RunConfig<S> {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = concurrency;
        self
    }

    pub fn with_request_middleware<M>(mut self, middleware: M) -> Self
    where
        M: RequestMiddleware<S> + 'static,
    {
        self.request_middlewares.push(Arc::new(middleware));
        self
    }

    pub fn with_response_middleware<M>(mut self, middleware: M) -> Self
    where
        M: ResponseMiddleware<S> + 'static,
    {
        self.response_middlewares.push(Arc::new(middleware));
        self
    }

    pub fn with_item_pipeline<P>(mut self, pipeline: P) -> Self
    where
        P: ItemPipeline<S> + 'static,
    {
        self.item_pipelines.push(Arc::new(pipeline));
        self
    }

    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = Some(timeout);
        self
    }

    pub fn with_log_stats_interval(mut self, interval: Duration) -> Self {
        self.log_stats_interval = Some(interval);
        self
    }

    pub fn with_max_pending_requests(mut self, max_pending_requests: usize) -> Self {
        self.max_pending_requests = Some(max_pending_requests);
        self
    }

    pub fn with_max_seen_requests(mut self, max_seen_requests: usize) -> Self {
        self.max_seen_requests = Some(max_seen_requests);
        self
    }

    pub fn with_unbounded_seen_requests(mut self) -> Self {
        self.max_seen_requests = None;
        self
    }

    pub fn with_html_max_size_bytes(mut self, html_max_size_bytes: usize) -> Self {
        self.html_max_size_bytes = html_max_size_bytes;
        self
    }

    pub fn with_keep_alive(mut self, keep_alive: bool) -> Self {
        self.keep_alive = keep_alive;
        self
    }

    pub fn with_middlewares<Req, Resp>(
        mut self,
        request_middlewares: Req,
        response_middlewares: Resp,
    ) -> Self
    where
        Req: IntoIterator<Item = Arc<dyn RequestMiddleware<S>>>,
        Resp: IntoIterator<Item = Arc<dyn ResponseMiddleware<S>>>,
    {
        self.request_middlewares.extend(request_middlewares);
        self.response_middlewares.extend(response_middlewares);
        self
    }
}

impl<S: Spider> From<RunConfig<S>> for EngineConfig<S> {
    fn from(config: RunConfig<S>) -> Self {
        EngineConfig {
            concurrency: config.concurrency,
            request_middlewares: config.request_middlewares,
            response_middlewares: config.response_middlewares,
            item_pipelines: config.item_pipelines,
            request_timeout: config.request_timeout,
            log_stats_interval: config.log_stats_interval,
            max_pending_requests: config.max_pending_requests,
            max_seen_requests: config.max_seen_requests,
            html_max_size_bytes: config.html_max_size_bytes,
            keep_alive: config.keep_alive,
        }
    }
}

#[inline]
pub async fn crawl<S: Spider>(spider: S) -> SilkwormResult<()> {
    crawl_with(spider, RunConfig::default()).await
}

pub async fn crawl_with<S: Spider>(spider: S, config: RunConfig<S>) -> SilkwormResult<()> {
    let engine = Engine::new(spider, config.into())?;
    engine.run().await
}

#[inline]
pub fn run_spider<S: Spider>(spider: S) -> SilkwormResult<()> {
    run_spider_with(spider, RunConfig::default())
}

pub fn run_spider_with<S: Spider>(spider: S, config: RunConfig<S>) -> SilkwormResult<()> {
    if tokio::runtime::Handle::try_current().is_ok() {
        return Err(SilkwormError::Config(
            "run_spider_with cannot run inside an existing Tokio runtime; use crawl_with instead"
                .to_string(),
        ));
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| SilkwormError::Config(err.to_string()))?;
    runtime.block_on(crawl_with(spider, config))
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_MAX_SEEN_REQUESTS, RunConfig};
    use crate::engine::EngineConfig;
    use crate::middlewares::{
        RequestMiddleware, ResponseMiddleware, RetryMiddleware, UserAgentMiddleware,
    };
    use crate::pipelines::JsonLinesPipeline;
    use crate::request::SpiderResult;
    use crate::response::HtmlResponse;
    use crate::spider::Spider;
    use std::sync::Arc;

    struct TestSpider;

    impl Spider for TestSpider {
        fn name(&self) -> &str {
            "test"
        }

        async fn parse(&self, _response: HtmlResponse<Self>) -> SpiderResult<Self> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn run_config_defaults() {
        let config: RunConfig<TestSpider> = RunConfig::default();
        assert_eq!(config.concurrency, 16);
        assert_eq!(config.html_max_size_bytes, 5_000_000);
        assert!(!config.keep_alive);
        assert!(config.request_timeout.is_none());
        assert!(config.log_stats_interval.is_none());
        assert!(config.max_pending_requests.is_none());
        assert_eq!(config.max_seen_requests, Some(DEFAULT_MAX_SEEN_REQUESTS));
    }

    #[test]
    fn run_config_into_engine_config_copies_fields() {
        let config = RunConfig::<TestSpider> {
            concurrency: 3,
            request_middlewares: Vec::new(),
            response_middlewares: Vec::new(),
            item_pipelines: Vec::new(),
            request_timeout: Some(std::time::Duration::from_secs(3)),
            log_stats_interval: Some(std::time::Duration::from_secs(1)),
            max_pending_requests: Some(9),
            max_seen_requests: Some(11),
            html_max_size_bytes: 123,
            keep_alive: true,
        };
        let engine_config: EngineConfig<TestSpider> = config.into();
        assert_eq!(engine_config.concurrency, 3);
        assert_eq!(
            engine_config.request_timeout,
            Some(std::time::Duration::from_secs(3))
        );
        assert_eq!(
            engine_config.log_stats_interval,
            Some(std::time::Duration::from_secs(1))
        );
        assert_eq!(engine_config.max_pending_requests, Some(9));
        assert_eq!(engine_config.max_seen_requests, Some(11));
        assert_eq!(engine_config.html_max_size_bytes, 123);
        assert!(engine_config.keep_alive);
    }

    #[test]
    fn run_config_builder_sets_fields() {
        let config = RunConfig::<TestSpider>::new()
            .with_concurrency(8)
            .with_request_timeout(std::time::Duration::from_secs(5))
            .with_log_stats_interval(std::time::Duration::from_secs(2))
            .with_max_pending_requests(12)
            .with_max_seen_requests(48)
            .with_html_max_size_bytes(321)
            .with_keep_alive(true);

        assert_eq!(config.concurrency, 8);
        assert_eq!(
            config.request_timeout,
            Some(std::time::Duration::from_secs(5))
        );
        assert_eq!(
            config.log_stats_interval,
            Some(std::time::Duration::from_secs(2))
        );
        assert_eq!(config.max_pending_requests, Some(12));
        assert_eq!(config.max_seen_requests, Some(48));
        assert_eq!(config.html_max_size_bytes, 321);
        assert!(config.keep_alive);
    }

    #[test]
    fn run_config_can_disable_seen_requests_bound() {
        let config = RunConfig::<TestSpider>::new().with_unbounded_seen_requests();
        assert!(config.max_seen_requests.is_none());
    }

    #[test]
    fn run_config_builder_adds_components() {
        let config = RunConfig::<TestSpider>::new()
            .with_request_middleware(UserAgentMiddleware::new(Vec::new(), None))
            .with_response_middleware(RetryMiddleware::new(1, None, None, 0.0))
            .with_item_pipeline(JsonLinesPipeline::new("data/test.jl"));

        assert_eq!(config.request_middlewares.len(), 1);
        assert_eq!(config.response_middlewares.len(), 1);
        assert_eq!(config.item_pipelines.len(), 1);
    }

    #[test]
    fn run_config_with_middlewares_appends_lists() {
        let request_middlewares: Vec<Arc<dyn RequestMiddleware<TestSpider>>> =
            vec![Arc::new(UserAgentMiddleware::new(Vec::new(), None))];
        let response_middlewares: Vec<Arc<dyn ResponseMiddleware<TestSpider>>> =
            vec![Arc::new(RetryMiddleware::new(1, None, None, 0.0))];

        let config = RunConfig::<TestSpider>::new()
            .with_middlewares(request_middlewares, response_middlewares);

        assert_eq!(config.request_middlewares.len(), 1);
        assert_eq!(config.response_middlewares.len(), 1);
    }

    #[tokio::test]
    async fn run_spider_with_returns_config_error_inside_runtime() {
        let result = super::run_spider_with(TestSpider, RunConfig::default());
        match result {
            Err(crate::errors::SilkwormError::Config(message)) => {
                assert!(message.contains("crawl_with"));
            }
            Err(other) => panic!("expected config error, got {other:?}"),
            Ok(_) => panic!("expected error, got ok"),
        }
    }

    #[test]
    fn run_spider_with_rejects_zero_max_pending_requests() {
        let config = RunConfig::<TestSpider>::new().with_max_pending_requests(0);
        let result = super::run_spider_with(TestSpider, config);
        match result {
            Err(crate::errors::SilkwormError::Config(message)) => {
                assert!(message.contains("max_pending_requests"));
            }
            Err(other) => panic!("expected config error, got {other:?}"),
            Ok(_) => panic!("expected error, got ok"),
        }
    }
}
