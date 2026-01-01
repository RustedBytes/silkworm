use std::sync::Arc;
use std::time::Duration;

use crate::engine::{Engine, EngineConfig};
use crate::errors::{SilkwormError, SilkwormResult};
use crate::middlewares::{RequestMiddleware, ResponseMiddleware};
use crate::pipelines::ItemPipeline;
use crate::spider::Spider;

pub struct RunConfig<S: Spider> {
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
            html_max_size_bytes: 5_000_000,
            keep_alive: false,
        }
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
            html_max_size_bytes: config.html_max_size_bytes,
            keep_alive: config.keep_alive,
        }
    }
}

pub async fn crawl<S: Spider>(spider: S) -> SilkwormResult<()> {
    crawl_with(spider, RunConfig::default()).await
}

pub async fn crawl_with<S: Spider>(spider: S, config: RunConfig<S>) -> SilkwormResult<()> {
    let engine = Engine::new(spider, config.into())?;
    engine.run().await
}

pub fn run_spider<S: Spider>(spider: S) -> SilkwormResult<()> {
    run_spider_with(spider, RunConfig::default())
}

pub fn run_spider_with<S: Spider>(spider: S, config: RunConfig<S>) -> SilkwormResult<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| SilkwormError::Config(err.to_string()))?;
    runtime.block_on(crawl_with(spider, config))
}

#[cfg(test)]
mod tests {
    use super::RunConfig;
    use crate::engine::EngineConfig;
    use crate::request::SpiderResult;
    use crate::response::HtmlResponse;
    use crate::spider::Spider;

    struct TestSpider;

    impl Spider for TestSpider {
        fn name(&self) -> &str {
            "test"
        }

        fn parse(
            &self,
            _response: HtmlResponse<Self>,
        ) -> impl std::future::Future<Output = SpiderResult<Self>> + Send + '_ {
            async { Vec::new() }
        }
    }

    #[test]
    fn run_config_defaults() {
        let config: RunConfig<TestSpider> = RunConfig::default();
        assert_eq!(config.concurrency, 16);
        assert_eq!(config.html_max_size_bytes, 5_000_000);
        assert_eq!(config.keep_alive, false);
        assert!(config.request_timeout.is_none());
        assert!(config.log_stats_interval.is_none());
        assert!(config.max_pending_requests.is_none());
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
        assert_eq!(engine_config.html_max_size_bytes, 123);
        assert_eq!(engine_config.keep_alive, true);
    }
}
