use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rand::Rng;
use tokio::time::sleep;

use crate::logging::get_logger;
use crate::request::{Callback, Request};
use crate::response::Response;
use crate::spider::Spider;
use crate::types::Item;

#[async_trait]
pub trait RequestMiddleware<S: Spider>: Send + Sync {
    async fn process_request(&self, request: Request<S>, spider: Arc<S>) -> Request<S>;
}

#[derive(Debug)]
pub enum ResponseAction<S> {
    Response(Response<S>),
    Request(Request<S>),
}

#[async_trait]
pub trait ResponseMiddleware<S: Spider>: Send + Sync {
    async fn process_response(&self, response: Response<S>, spider: Arc<S>) -> ResponseAction<S>;
}

pub struct UserAgentMiddleware {
    user_agents: Vec<String>,
    default: String,
    logger: crate::logging::Logger,
}

impl UserAgentMiddleware {
    pub fn new(user_agents: Vec<String>, default: Option<String>) -> Self {
        UserAgentMiddleware {
            user_agents,
            default: default.unwrap_or_else(|| "silkworm/0.1".to_string()),
            logger: get_logger("UserAgentMiddleware", None),
        }
    }
}

#[async_trait]
impl<S: Spider> RequestMiddleware<S> for UserAgentMiddleware {
    async fn process_request(&self, mut request: Request<S>, _spider: Arc<S>) -> Request<S> {
        let has_header = request
            .headers
            .keys()
            .any(|key| key.eq_ignore_ascii_case("user-agent"));
        if !has_header {
            let ua = if self.user_agents.is_empty() {
                self.default.clone()
            } else {
                let mut rng = rand::thread_rng();
                let idx = rng.gen_range(0..self.user_agents.len());
                self.user_agents[idx].clone()
            };
            request.headers.insert("User-Agent".to_string(), ua.clone());
            self.logger
                .debug("Assigned user agent", &[("user_agent", ua)]);
        }
        request
    }
}

pub struct ProxyMiddleware {
    proxies: Vec<String>,
    random_selection: bool,
    index: std::sync::Mutex<usize>,
    logger: crate::logging::Logger,
}

impl ProxyMiddleware {
    pub fn new(proxies: Vec<String>, random_selection: bool) -> Self {
        ProxyMiddleware {
            proxies,
            random_selection,
            index: std::sync::Mutex::new(0),
            logger: get_logger("ProxyMiddleware", None),
        }
    }

    pub fn from_file(path: impl AsRef<Path>, random_selection: bool) -> std::io::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let proxies = content
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .map(|line| line.to_string())
            .collect();
        Ok(Self::new(proxies, random_selection))
    }
}

#[async_trait]
impl<S: Spider> RequestMiddleware<S> for ProxyMiddleware {
    async fn process_request(&self, mut request: Request<S>, _spider: Arc<S>) -> Request<S> {
        if self.proxies.is_empty() {
            return request;
        }
        let proxy = if self.random_selection {
            let mut rng = rand::thread_rng();
            let idx = rng.gen_range(0..self.proxies.len());
            self.proxies[idx].clone()
        } else {
            let mut guard = self.index.lock().expect("proxy index lock");
            let proxy = self.proxies[*guard % self.proxies.len()].clone();
            *guard = (*guard + 1) % self.proxies.len();
            proxy
        };
        request
            .meta
            .insert("proxy".to_string(), Item::from(proxy.clone()));
        self.logger.debug("Assigned proxy", &[("proxy", proxy)]);
        request
    }
}

pub struct RetryMiddleware {
    max_times: u64,
    retry_http_codes: Vec<u16>,
    sleep_http_codes: Vec<u16>,
    backoff_base: f64,
    logger: crate::logging::Logger,
}

impl RetryMiddleware {
    pub fn new(
        max_times: u64,
        retry_http_codes: Option<Vec<u16>>,
        sleep_http_codes: Option<Vec<u16>>,
        backoff_base: f64,
    ) -> Self {
        let retry_http_codes =
            retry_http_codes.unwrap_or_else(|| vec![500, 502, 503, 504, 522, 524, 408, 429]);
        let sleep_http_codes = sleep_http_codes
            .clone()
            .unwrap_or_else(|| retry_http_codes.clone());
        let mut merged = retry_http_codes.clone();
        for code in &sleep_http_codes {
            if !merged.contains(code) {
                merged.push(*code);
            }
        }
        RetryMiddleware {
            max_times,
            retry_http_codes: merged,
            sleep_http_codes,
            backoff_base,
            logger: get_logger("RetryMiddleware", None),
        }
    }
}

#[async_trait]
impl<S: Spider> ResponseMiddleware<S> for RetryMiddleware {
    async fn process_response(&self, response: Response<S>, _spider: Arc<S>) -> ResponseAction<S> {
        let status = response.status;
        if !self.retry_http_codes.contains(&status) {
            return ResponseAction::Response(response);
        }

        let retry_times = response
            .request
            .meta
            .get("retry_times")
            .and_then(|value| value.as_u64())
            .unwrap_or(0);

        if retry_times >= self.max_times {
            return ResponseAction::Response(response);
        }

        let mut req = response.request.clone();
        req.dont_filter = true;
        req.meta
            .insert("retry_times".to_string(), Item::from(retry_times + 1));

        let delay = self.backoff_base * 2f64.powi(retry_times as i32);
        self.logger.warn(
            "Retrying request",
            &[
                ("url", req.url.clone()),
                ("delay", format!("{:.2}", delay)),
                ("attempt", (retry_times + 1).to_string()),
                ("status", status.to_string()),
            ],
        );

        if self.sleep_http_codes.contains(&status) && delay > 0.0 {
            sleep(Duration::from_secs_f64(delay)).await;
        }

        ResponseAction::Request(req)
    }
}

type DelayStrategyFn<S> = Arc<dyn Fn(&Request<S>, &S) -> f64 + Send + Sync>;

enum DelayStrategy<S: Spider> {
    Fixed(f64),
    Random(f64, f64),
    Custom(DelayStrategyFn<S>),
}

pub struct DelayMiddleware<S: Spider> {
    strategy: DelayStrategy<S>,
    logger: crate::logging::Logger,
}

impl<S: Spider> DelayMiddleware<S> {
    pub fn fixed(delay: f64) -> Self {
        DelayMiddleware {
            strategy: DelayStrategy::Fixed(delay),
            logger: get_logger("DelayMiddleware", None),
        }
    }

    pub fn random(min_delay: f64, max_delay: f64) -> Self {
        DelayMiddleware {
            strategy: DelayStrategy::Random(min_delay, max_delay),
            logger: get_logger("DelayMiddleware", None),
        }
    }

    pub fn custom<F>(func: F) -> Self
    where
        F: Fn(&Request<S>, &S) -> f64 + Send + Sync + 'static,
    {
        DelayMiddleware {
            strategy: DelayStrategy::Custom(Arc::new(func)),
            logger: get_logger("DelayMiddleware", None),
        }
    }
}

#[async_trait]
impl<S: Spider> RequestMiddleware<S> for DelayMiddleware<S> {
    async fn process_request(&self, request: Request<S>, spider: Arc<S>) -> Request<S> {
        let delay = match &self.strategy {
            DelayStrategy::Fixed(value) => *value,
            DelayStrategy::Random(min_delay, max_delay) => {
                if max_delay <= min_delay {
                    *min_delay
                } else {
                    let mut rng = rand::thread_rng();
                    rng.gen_range(*min_delay..*max_delay)
                }
            }
            DelayStrategy::Custom(func) => func(&request, spider.as_ref()),
        };

        if delay > 0.0 {
            self.logger.debug(
                "Delaying request",
                &[
                    ("url", request.url.clone()),
                    ("delay", format!("{:.3}", delay)),
                ],
            );
            sleep(Duration::from_secs_f64(delay)).await;
        }

        request
    }
}

pub struct SkipNonHtmlMiddleware {
    allowed_types: Vec<String>,
    sniff_bytes: usize,
    logger: crate::logging::Logger,
}

impl SkipNonHtmlMiddleware {
    pub fn new(allowed_types: Option<Vec<String>>, sniff_bytes: usize) -> Self {
        SkipNonHtmlMiddleware {
            allowed_types: allowed_types.unwrap_or_else(|| vec!["html".to_string()]),
            sniff_bytes,
            logger: get_logger("SkipNonHtmlMiddleware", None),
        }
    }
}

#[async_trait]
impl<S: Spider> ResponseMiddleware<S> for SkipNonHtmlMiddleware {
    async fn process_response(
        &self,
        mut response: Response<S>,
        _spider: Arc<S>,
    ) -> ResponseAction<S> {
        if response
            .request
            .meta
            .get("allow_non_html")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            return ResponseAction::Response(response);
        }

        if looks_like_html(&response, &self.allowed_types, self.sniff_bytes) {
            return ResponseAction::Response(response);
        }

        self.logger.info(
            "Skipping non-HTML response",
            &[
                ("url", response.url.clone()),
                ("status", response.status.to_string()),
                (
                    "content_type",
                    response
                        .headers
                        .get("content-type")
                        .cloned()
                        .unwrap_or_else(|| "unknown".to_string()),
                ),
            ],
        );

        response.request.callback = Some(noop_callback());
        ResponseAction::Response(response)
    }
}

fn looks_like_html<S: Spider>(
    response: &Response<S>,
    allowed_types: &[String],
    sniff_bytes: usize,
) -> bool {
    let content_type = response
        .headers
        .get("content-type")
        .map(|value| value.to_lowercase())
        .unwrap_or_default();

    if allowed_types
        .iter()
        .any(|token| content_type.contains(token))
    {
        return true;
    }

    if sniff_bytes == 0 {
        return false;
    }

    let len = std::cmp::min(sniff_bytes, response.body.len());
    let snippet = &response.body[..len];
    contains_html_marker(snippet)
}

fn noop_callback<S: Spider>() -> Callback<S> {
    Arc::new(|_spider, _response| Box::pin(async move { Vec::new() }))
}

fn contains_html_marker(snippet: &[u8]) -> bool {
    contains_ascii_case_insensitive(snippet, b"<html")
        || contains_ascii_case_insensitive(snippet, b"<!doctype")
}

fn contains_ascii_case_insensitive(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|window| {
        window
            .iter()
            .zip(needle.iter())
            .all(|(a, b)| a.to_ascii_lowercase() == b.to_ascii_lowercase())
    })
}

#[cfg(test)]
mod tests {
    use super::{
        DelayMiddleware, ProxyMiddleware, RequestMiddleware, ResponseAction, ResponseMiddleware,
        RetryMiddleware, SkipNonHtmlMiddleware, UserAgentMiddleware,
    };
    use crate::request::{Request, SpiderResult};
    use crate::response::{HtmlResponse, Response};
    use crate::spider::Spider;
    use crate::types::Headers;
    use std::sync::Arc;

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

    fn base_response(request: Request<TestSpider>, status: u16) -> Response<TestSpider> {
        Response {
            url: request.url.clone(),
            status,
            headers: Headers::new(),
            body: Vec::new(),
            request,
        }
    }

    #[tokio::test]
    async fn user_agent_middleware_sets_header_when_missing() {
        let middleware = UserAgentMiddleware::new(Vec::new(), None);
        let request = Request::<TestSpider>::new("https://example.com");
        let spider = Arc::new(TestSpider);

        let request = middleware.process_request(request, spider).await;

        assert_eq!(
            request.headers.get("User-Agent").map(String::as_str),
            Some("silkworm/0.1")
        );
    }

    #[tokio::test]
    async fn proxy_middleware_cycles_when_not_random() {
        let middleware = ProxyMiddleware::new(
            vec!["http://proxy-a".to_string(), "http://proxy-b".to_string()],
            false,
        );
        let spider = Arc::new(TestSpider);

        let req1 = middleware
            .process_request(Request::new("https://example.com"), spider.clone())
            .await;
        let req2 = middleware
            .process_request(Request::new("https://example.com"), spider)
            .await;

        assert_eq!(
            req1.meta
                .get("proxy")
                .and_then(|v: &serde_json::Value| v.as_str()),
            Some("http://proxy-a")
        );
        assert_eq!(
            req2.meta
                .get("proxy")
                .and_then(|v: &serde_json::Value| v.as_str()),
            Some("http://proxy-b")
        );
    }

    #[tokio::test]
    async fn retry_middleware_returns_request_on_retryable_status() {
        let middleware = RetryMiddleware::new(1, Some(vec![500]), Some(vec![500]), 0.0);
        let spider = Arc::new(TestSpider);
        let request = Request::<TestSpider>::new("https://example.com");
        let response = base_response(request, 500);

        let action = middleware.process_response(response, spider).await;

        match action {
            ResponseAction::Request(req) => {
                assert!(req.dont_filter);
                assert_eq!(
                    req.meta.get("retry_times").and_then(|v| v.as_u64()),
                    Some(1)
                );
            }
            ResponseAction::Response(_) => panic!("expected retry request"),
        }
    }

    #[tokio::test]
    async fn skip_non_html_sets_noop_callback() {
        let middleware = SkipNonHtmlMiddleware::new(None, 0);
        let spider = Arc::new(TestSpider);
        let mut request = Request::<TestSpider>::new("https://example.com");
        request
            .headers
            .insert("Accept".to_string(), "application/json".to_string());
        let mut response = base_response(request, 200);
        response
            .headers
            .insert("content-type".to_string(), "application/json".to_string());

        let action = middleware.process_response(response, spider).await;

        match action {
            ResponseAction::Response(response) => {
                assert!(response.request.callback.is_some());
            }
            ResponseAction::Request(_) => panic!("expected response"),
        }
    }

    #[tokio::test]
    async fn delay_middleware_custom_keeps_request_intact() {
        let middleware = DelayMiddleware::custom(|_, _| 0.0);
        let spider = Arc::new(TestSpider);
        let request = Request::<TestSpider>::new("https://example.com");

        let delayed = middleware.process_request(request.clone(), spider).await;

        assert_eq!(delayed.url, request.url);
        assert_eq!(delayed.method, request.method);
        assert_eq!(delayed.headers.len(), request.headers.len());
    }
}
