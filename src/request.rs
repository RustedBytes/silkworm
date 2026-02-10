use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use serde_json::Number;

use crate::errors::SilkwormResult;
use crate::response::Response;
use crate::types::{Headers, Item, Meta, Params};

pub type CallbackFuture<S> = Pin<Box<dyn Future<Output = SpiderResult<S>> + Send>>;
pub type Callback<S> = Arc<dyn Fn(Arc<S>, Response<S>) -> CallbackFuture<S> + Send + Sync>;

pub mod meta_keys {
    pub const ALLOW_NON_HTML: &str = "allow_non_html";
    pub const PROXY: &str = "proxy";
    pub const RETRY_TIMES: &str = "retry_times";
    pub const RETRY_DELAY_SECS: &str = "retry_delay_secs";
    pub const REQUEST_DELAY_SECS: &str = "request_delay_secs";
    pub const REQUEST_DELAY_SCHEDULED: &str = "request_delay_scheduled";
    pub const REDIRECT_TIMES: &str = "redirect_times";
}

pub struct Request<S> {
    pub url: String,
    pub method: String,
    pub headers: Headers,
    pub params: Params,
    pub data: Option<Bytes>,
    pub json: Option<Item>,
    pub meta: Meta,
    pub timeout: Option<Duration>,
    pub callback: Option<Callback<S>>,
    pub dont_filter: bool,
    pub priority: i32,
}

impl<S> Clone for Request<S> {
    fn clone(&self) -> Self {
        Request {
            url: self.url.clone(),
            method: self.method.clone(),
            headers: self.headers.clone(),
            params: self.params.clone(),
            data: self.data.clone(),
            json: self.json.clone(),
            meta: self.meta.clone(),
            timeout: self.timeout,
            callback: self.callback.clone(),
            dont_filter: self.dont_filter,
            priority: self.priority,
        }
    }
}

impl<S> fmt::Debug for Request<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let data_len = self.data.as_ref().map(Bytes::len);
        let has_callback = self.callback.is_some();
        f.debug_struct("Request")
            .field("url", &self.url)
            .field("method", &self.method)
            .field("headers", &self.headers)
            .field("params", &self.params)
            .field("data_len", &data_len)
            .field("json", &self.json)
            .field("meta", &self.meta)
            .field("timeout", &self.timeout)
            .field("callback", &has_callback)
            .field("dont_filter", &self.dont_filter)
            .field("priority", &self.priority)
            .finish()
    }
}

impl<S> Request<S> {
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Request {
            url: url.into(),
            method: "GET".to_string(),
            headers: Headers::new(),
            params: Params::new(),
            data: None,
            json: None,
            meta: Meta::new(),
            timeout: None,
            callback: None,
            dont_filter: false,
            priority: 0,
        }
    }

    #[inline]
    #[must_use]
    pub fn builder(url: impl Into<String>) -> RequestBuilder<S> {
        RequestBuilder::new(url)
    }

    #[inline]
    #[must_use]
    pub fn get(url: impl Into<String>) -> Self {
        Self::new(url)
    }

    #[inline]
    #[must_use]
    pub fn post(url: impl Into<String>) -> Self {
        Self::new(url).with_method("POST")
    }

    #[must_use]
    pub fn with_method(mut self, method: impl Into<String>) -> Self {
        self.method = method.into();
        self
    }

    #[must_use]
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    #[must_use]
    pub fn with_headers<I, K, V>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (key, value) in headers {
            self.headers.insert(key.into(), value.into());
        }
        self
    }

    #[must_use]
    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.insert(key.into(), value.into());
        self
    }

    #[must_use]
    pub fn with_params<I, K, V>(mut self, params: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (key, value) in params {
            self.params.insert(key.into(), value.into());
        }
        self
    }

    #[must_use]
    pub fn with_data(mut self, data: impl Into<Bytes>) -> Self {
        self.data = Some(data.into());
        self
    }

    #[must_use]
    pub fn with_json(mut self, json: Item) -> Self {
        self.json = Some(json);
        self
    }

    #[must_use]
    pub fn with_meta(mut self, key: impl Into<String>, value: Item) -> Self {
        self.meta.insert(key.into(), value);
        self
    }

    #[must_use]
    pub fn with_meta_str(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.meta.insert(key.into(), Item::String(value.into()));
        self
    }

    #[must_use]
    pub fn with_meta_bool(mut self, key: impl Into<String>, value: bool) -> Self {
        self.meta.insert(key.into(), Item::Bool(value));
        self
    }

    #[inline]
    #[must_use]
    pub fn with_allow_non_html(self, allow_non_html: bool) -> Self {
        self.with_meta_bool(meta_keys::ALLOW_NON_HTML, allow_non_html)
    }

    #[must_use]
    pub fn with_meta_number<N>(mut self, key: impl Into<String>, value: N) -> Self
    where
        N: Into<Number>,
    {
        self.meta.insert(key.into(), Item::Number(value.into()));
        self
    }

    #[must_use]
    pub fn with_meta_entries<I, K>(mut self, meta: I) -> Self
    where
        I: IntoIterator<Item = (K, Item)>,
        K: Into<String>,
    {
        for (key, value) in meta {
            self.meta.insert(key.into(), value);
        }
        self
    }

    #[inline]
    pub fn meta_value(&self, key: &str) -> Option<&Item> {
        self.meta.get(key)
    }

    #[inline]
    pub fn meta_str(&self, key: &str) -> Option<&str> {
        self.meta.get(key).and_then(Item::as_str)
    }

    #[inline]
    pub fn meta_bool(&self, key: &str) -> Option<bool> {
        self.meta.get(key).and_then(Item::as_bool)
    }

    #[inline]
    pub fn meta_u64(&self, key: &str) -> Option<u64> {
        self.meta.get(key).and_then(Item::as_u64)
    }

    #[inline]
    pub fn meta_f64(&self, key: &str) -> Option<f64> {
        self.meta.get(key).and_then(Item::as_f64)
    }

    #[inline]
    pub fn take_meta_u64(&mut self, key: &str) -> Option<u64> {
        self.meta.remove(key).and_then(|value| Item::as_u64(&value))
    }

    #[inline]
    pub fn take_meta_f64(&mut self, key: &str) -> Option<f64> {
        self.meta.remove(key).and_then(|value| Item::as_f64(&value))
    }

    #[inline]
    pub fn take_meta_bool(&mut self, key: &str) -> Option<bool> {
        self.meta
            .remove(key)
            .and_then(|value| Item::as_bool(&value))
    }

    #[inline]
    pub fn proxy(&self) -> Option<&str> {
        self.meta_str(meta_keys::PROXY)
    }

    #[inline]
    #[must_use]
    pub fn with_proxy(self, proxy: impl Into<String>) -> Self {
        self.with_meta_str(meta_keys::PROXY, proxy)
    }

    #[inline]
    pub fn allow_non_html(&self) -> bool {
        self.meta_bool(meta_keys::ALLOW_NON_HTML).unwrap_or(false)
    }

    #[inline]
    pub fn retry_times(&self) -> u64 {
        self.meta_u64(meta_keys::RETRY_TIMES).unwrap_or(0)
    }

    #[inline]
    pub fn set_retry_times(&mut self, retry_times: u64) {
        self.meta
            .insert(meta_keys::RETRY_TIMES.to_string(), Item::from(retry_times));
    }

    #[inline]
    pub fn increment_retry_times(&mut self) -> u64 {
        let next = self.retry_times().saturating_add(1);
        self.set_retry_times(next);
        next
    }

    #[inline]
    pub fn retry_delay_secs(&self) -> Option<f64> {
        self.meta_f64(meta_keys::RETRY_DELAY_SECS)
    }

    #[inline]
    pub fn set_retry_delay_secs(&mut self, delay_secs: f64) {
        if delay_secs > 0.0 {
            self.meta.insert(
                meta_keys::RETRY_DELAY_SECS.to_string(),
                Item::from(delay_secs),
            );
        } else {
            self.meta.remove(meta_keys::RETRY_DELAY_SECS);
        }
    }

    #[inline]
    pub fn take_retry_delay_secs(&mut self) -> Option<f64> {
        self.take_meta_f64(meta_keys::RETRY_DELAY_SECS)
            .filter(|delay_secs| *delay_secs > 0.0)
    }

    #[inline]
    pub fn request_delay_secs(&self) -> Option<f64> {
        self.meta_f64(meta_keys::REQUEST_DELAY_SECS)
    }

    #[inline]
    pub fn set_request_delay_secs(&mut self, delay_secs: f64) {
        if delay_secs > 0.0 {
            self.meta.insert(
                meta_keys::REQUEST_DELAY_SECS.to_string(),
                Item::from(delay_secs),
            );
        } else {
            self.meta.remove(meta_keys::REQUEST_DELAY_SECS);
        }
    }

    #[inline]
    pub fn take_request_delay_secs(&mut self) -> Option<f64> {
        self.take_meta_f64(meta_keys::REQUEST_DELAY_SECS)
            .filter(|delay_secs| *delay_secs > 0.0)
    }

    #[inline]
    pub fn mark_request_delay_scheduled(&mut self) {
        self.meta.insert(
            meta_keys::REQUEST_DELAY_SCHEDULED.to_string(),
            Item::Bool(true),
        );
    }

    #[inline]
    pub fn take_request_delay_scheduled(&mut self) -> bool {
        self.take_meta_bool(meta_keys::REQUEST_DELAY_SCHEDULED)
            .unwrap_or(false)
    }

    #[inline]
    pub fn redirect_times(&self) -> u64 {
        self.meta_u64(meta_keys::REDIRECT_TIMES).unwrap_or(0)
    }

    #[inline]
    pub fn set_redirect_times(&mut self, redirect_times: u64) {
        self.meta.insert(
            meta_keys::REDIRECT_TIMES.to_string(),
            Item::from(redirect_times),
        );
    }

    #[inline]
    pub fn increment_redirect_times(&mut self) -> u64 {
        let next = self.redirect_times().saturating_add(1);
        self.set_redirect_times(next);
        next
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    #[must_use]
    pub fn with_callback(mut self, callback: Callback<S>) -> Self {
        self.callback = Some(callback);
        self
    }

    #[must_use]
    pub fn with_callback_fn<F, Fut>(mut self, func: F) -> Self
    where
        S: Send + Sync + 'static,
        F: Fn(Arc<S>, Response<S>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = SpiderResult<S>> + Send + 'static,
    {
        self.callback = Some(callback_from(func));
        self
    }

    #[must_use]
    pub fn with_dont_filter(mut self, dont_filter: bool) -> Self {
        self.dont_filter = dont_filter;
        self
    }

    #[must_use]
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    #[must_use]
    pub fn replace<F>(&self, updater: F) -> Self
    where
        F: FnOnce(&mut Request<S>),
    {
        let mut cloned = self.clone();
        updater(&mut cloned);
        cloned
    }
}

pub struct RequestBuilder<S> {
    request: Request<S>,
}

impl<S> RequestBuilder<S> {
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        RequestBuilder {
            request: Request::new(url),
        }
    }

    #[must_use]
    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.request.headers.insert(key.into(), value.into());
        self
    }

    #[must_use]
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.request.params.insert(key.into(), value.into());
        self
    }

    #[must_use]
    pub fn json(mut self, value: impl Into<Item>) -> Self {
        self.request.json = Some(value.into());
        self
    }

    #[must_use]
    pub fn data(mut self, data: impl Into<Bytes>) -> Self {
        self.request.data = Some(data.into());
        self
    }

    #[must_use]
    pub fn callback_fn<F, Fut>(mut self, func: F) -> Self
    where
        S: Send + Sync + 'static,
        F: Fn(Arc<S>, Response<S>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = SpiderResult<S>> + Send + 'static,
    {
        self.request.callback = Some(callback_from(func));
        self
    }

    #[must_use]
    pub fn dont_filter(mut self, dont_filter: bool) -> Self {
        self.request.dont_filter = dont_filter;
        self
    }

    #[must_use]
    pub fn priority(mut self, priority: i32) -> Self {
        self.request.priority = priority;
        self
    }

    #[must_use]
    pub fn allow_non_html(mut self, allow_non_html: bool) -> Self {
        self.request.meta.insert(
            meta_keys::ALLOW_NON_HTML.to_string(),
            Item::Bool(allow_non_html),
        );
        self
    }

    #[inline]
    #[must_use]
    pub fn build(self) -> Request<S> {
        self.request
    }
}

#[derive(Clone)]
pub enum SpiderOutput<S> {
    Request(Box<Request<S>>),
    Item(Item),
}

pub type SpiderOutputs<S> = Vec<SpiderOutput<S>>;
pub type SpiderResult<S> = SilkwormResult<SpiderOutputs<S>>;

impl<S> From<Request<S>> for SpiderOutput<S> {
    fn from(value: Request<S>) -> Self {
        SpiderOutput::Request(Box::new(value))
    }
}

impl<S> From<Item> for SpiderOutput<S> {
    fn from(value: Item) -> Self {
        SpiderOutput::Item(value)
    }
}

#[must_use]
pub fn callback_from<S, F, Fut>(func: F) -> Callback<S>
where
    S: Send + Sync + 'static,
    F: Fn(Arc<S>, Response<S>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = SpiderResult<S>> + Send + 'static,
{
    Arc::new(move |spider: Arc<S>, response: Response<S>| {
        let fut = func(spider, response);
        Box::pin(fut)
    })
}

#[inline]
#[must_use]
pub fn callback_from_fn<S, Fut>(func: fn(Arc<S>, Response<S>) -> Fut) -> Callback<S>
where
    S: Send + Sync + 'static,
    Fut: Future<Output = SpiderResult<S>> + Send + 'static,
{
    callback_from(func)
}

#[cfg(test)]
mod tests {
    use super::{Request, SpiderResult, callback_from, callback_from_fn, meta_keys};
    use crate::response::Response;
    use crate::types::{Headers, Item};
    use bytes::Bytes;
    use std::sync::Arc;

    struct TestSpider;

    #[test]
    fn request_replace_updates_clone() {
        let req = Request::<()>::new("https://example.com");
        let updated = req.replace(|r| {
            r.url = "https://example.com/next".to_string();
            r.priority = 10;
        });

        assert_eq!(req.url, "https://example.com");
        assert_eq!(updated.url, "https://example.com/next");
        assert_eq!(updated.priority, 10);
    }

    #[test]
    fn request_get_and_post_set_method() {
        let get = Request::<()>::get("https://example.com");
        let post = Request::<()>::post("https://example.com");
        assert_eq!(get.method, "GET");
        assert_eq!(post.method, "POST");
    }

    #[test]
    fn request_bulk_inserts_headers_params_and_meta() {
        let req = Request::<()>::new("https://example.com")
            .with_headers([("Accept", "text/html"), ("X-Trace", "1")])
            .with_params([("q", "rust"), ("page", "2")])
            .with_meta_entries([("trace", Item::from("abc"))]);

        assert_eq!(
            req.headers.get("Accept").map(String::as_str),
            Some("text/html")
        );
        assert_eq!(req.headers.get("X-Trace").map(String::as_str), Some("1"));
        assert_eq!(req.params.get("q").map(String::as_str), Some("rust"));
        assert_eq!(req.params.get("page").map(String::as_str), Some("2"));
        assert_eq!(req.meta.get("trace").and_then(|v| v.as_str()), Some("abc"));
    }

    #[test]
    fn request_meta_helpers_set_common_types() {
        let req = Request::<()>::new("https://example.com")
            .with_meta_str("trace", "abc")
            .with_meta_bool("flag", true)
            .with_meta_number("count", 7u64);

        assert_eq!(req.meta.get("trace").and_then(|v| v.as_str()), Some("abc"));
        assert_eq!(req.meta.get("flag").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(req.meta.get("count").and_then(|v| v.as_u64()), Some(7));
    }

    #[test]
    fn request_typed_meta_accessors_work() {
        let mut req = Request::<()>::new("https://example.com")
            .with_meta_str("trace", "abc")
            .with_meta_bool("flag", true)
            .with_meta_number("count", 7u64)
            .with_meta("delay", Item::from(0.5f64));

        assert_eq!(req.meta_str("trace"), Some("abc"));
        assert_eq!(req.meta_bool("flag"), Some(true));
        assert_eq!(req.meta_u64("count"), Some(7));
        assert_eq!(req.meta_f64("delay"), Some(0.5));
        assert_eq!(req.take_meta_u64("count"), Some(7));
        assert_eq!(req.take_meta_f64("delay"), Some(0.5));
        assert!(req.meta_u64("count").is_none());
        assert!(req.meta_f64("delay").is_none());
    }

    #[test]
    fn request_contract_meta_helpers_work() {
        let mut req = Request::<()>::new("https://example.com")
            .with_proxy("http://proxy.local:8080")
            .with_allow_non_html(true);

        assert_eq!(req.proxy(), Some("http://proxy.local:8080"));
        assert!(req.allow_non_html());
        assert_eq!(req.retry_times(), 0);
        assert_eq!(req.increment_retry_times(), 1);
        assert_eq!(req.retry_times(), 1);

        req.set_retry_delay_secs(0.75);
        assert_eq!(req.retry_delay_secs(), Some(0.75));
        assert_eq!(req.take_retry_delay_secs(), Some(0.75));
        assert!(req.retry_delay_secs().is_none());

        req.set_request_delay_secs(0.2);
        assert_eq!(req.request_delay_secs(), Some(0.2));
        assert_eq!(req.take_request_delay_secs(), Some(0.2));
        assert!(req.request_delay_secs().is_none());
        req.mark_request_delay_scheduled();
        assert!(req.take_request_delay_scheduled());
        assert!(!req.take_request_delay_scheduled());

        assert_eq!(req.redirect_times(), 0);
        assert_eq!(req.increment_redirect_times(), 1);
        assert_eq!(req.redirect_times(), 1);
    }

    #[test]
    fn request_builder_sets_fields() {
        let req = Request::<()>::builder("https://example.com/search")
            .header("Accept", "text/html")
            .param("q", "rust")
            .json(Item::from(1))
            .data(vec![1, 2, 3])
            .allow_non_html(true)
            .dont_filter(true)
            .priority(9)
            .build();

        assert_eq!(req.url, "https://example.com/search");
        assert_eq!(
            req.headers.get("Accept").map(String::as_str),
            Some("text/html")
        );
        assert_eq!(req.params.get("q").map(String::as_str), Some("rust"));
        assert_eq!(req.json.as_ref().and_then(|v| v.as_i64()), Some(1));
        assert_eq!(req.data.as_ref().map(Bytes::len), Some(3));
        assert_eq!(
            req.meta
                .get(meta_keys::ALLOW_NON_HTML)
                .and_then(|value| value.as_bool()),
            Some(true)
        );
        assert!(req.dont_filter);
        assert_eq!(req.priority, 9);
    }

    #[test]
    fn request_allow_non_html_sets_meta() {
        let req = Request::<()>::new("https://example.com").with_allow_non_html(true);
        assert_eq!(
            req.meta
                .get(meta_keys::ALLOW_NON_HTML)
                .and_then(|value| value.as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn request_builder_callback_sets_closure() {
        let req = Request::<TestSpider>::builder("https://example.com")
            .callback_fn(
                |_spider: Arc<TestSpider>, _response: Response<TestSpider>| async {
                    Ok(Vec::new())
                },
            )
            .build();

        assert!(req.callback.is_some());
    }

    #[tokio::test]
    async fn callback_from_fn_wraps_function() {
        async fn handler(
            _spider: Arc<TestSpider>,
            _response: Response<TestSpider>,
        ) -> SpiderResult<TestSpider> {
            Ok(vec![Item::from("ok").into()])
        }

        let callback = callback_from_fn(handler);
        let request = Request::<TestSpider>::new("https://example.com");
        let response = Response {
            url: "https://example.com".to_string(),
            status: 200,
            headers: Headers::new(),
            body: Bytes::new(),
            request,
        };

        let outputs = callback(Arc::new(TestSpider), response)
            .await
            .expect("callback outputs");
        assert_eq!(outputs.len(), 1);
    }

    #[tokio::test]
    async fn callback_from_accepts_closure() {
        let suffix = "ok".to_string();
        let callback = callback_from(
            move |_spider: Arc<TestSpider>, _response: Response<TestSpider>| {
                let suffix = suffix.clone();
                async move { Ok(vec![Item::from(suffix).into()]) }
            },
        );

        let request = Request::<TestSpider>::new("https://example.com");
        let response = Response {
            url: "https://example.com".to_string(),
            status: 200,
            headers: Headers::new(),
            body: Bytes::new(),
            request,
        };

        let outputs = callback(Arc::new(TestSpider), response)
            .await
            .expect("callback outputs");
        assert_eq!(outputs.len(), 1);
    }

    #[tokio::test]
    async fn request_with_callback_fn_accepts_closure() {
        let request = Request::<TestSpider>::new("https://example.com").with_callback_fn(
            |_spider: Arc<TestSpider>, _response: Response<TestSpider>| async {
                Ok(vec![Item::from("ok").into()])
            },
        );

        let callback = request.callback.clone().expect("callback");
        let response = Response {
            url: "https://example.com".to_string(),
            status: 200,
            headers: Headers::new(),
            body: Bytes::new(),
            request,
        };

        let outputs = callback(Arc::new(TestSpider), response)
            .await
            .expect("callback outputs");
        assert_eq!(outputs.len(), 1);
    }
}
