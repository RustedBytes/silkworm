use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Number;

use crate::response::Response;
use crate::types::{Headers, Item, Meta, Params};

pub type CallbackFuture<S> = Pin<Box<dyn Future<Output = SpiderResult<S>> + Send>>;
pub type Callback<S> = Arc<dyn Fn(Arc<S>, Response<S>) -> CallbackFuture<S> + Send + Sync>;

pub struct Request<S> {
    pub url: String,
    pub method: String,
    pub headers: Headers,
    pub params: Params,
    pub data: Option<Vec<u8>>,
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
        let data_len = self.data.as_ref().map(|data| data.len());
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

    pub fn builder(url: impl Into<String>) -> RequestBuilder<S> {
        RequestBuilder::new(url)
    }

    pub fn get(url: impl Into<String>) -> Self {
        Self::new(url)
    }

    pub fn post(url: impl Into<String>) -> Self {
        Self::new(url).with_method("POST")
    }

    pub fn with_method(mut self, method: impl Into<String>) -> Self {
        self.method = method.into();
        self
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

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

    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.insert(key.into(), value.into());
        self
    }

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

    pub fn with_data(mut self, data: Vec<u8>) -> Self {
        self.data = Some(data);
        self
    }

    pub fn with_json(mut self, json: Item) -> Self {
        self.json = Some(json);
        self
    }

    pub fn with_meta(mut self, key: impl Into<String>, value: Item) -> Self {
        self.meta.insert(key.into(), value);
        self
    }

    pub fn with_meta_str(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.meta
            .insert(key.into(), Item::String(value.into()));
        self
    }

    pub fn with_meta_bool(mut self, key: impl Into<String>, value: bool) -> Self {
        self.meta.insert(key.into(), Item::Bool(value));
        self
    }

    pub fn with_meta_number<N>(mut self, key: impl Into<String>, value: N) -> Self
    where
        N: Into<Number>,
    {
        self.meta
            .insert(key.into(), Item::Number(value.into()));
        self
    }

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

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn with_callback(mut self, callback: Callback<S>) -> Self {
        self.callback = Some(callback);
        self
    }

    pub fn with_callback_fn<F, Fut>(mut self, func: F) -> Self
    where
        S: Send + Sync + 'static,
        F: Fn(Arc<S>, Response<S>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = SpiderResult<S>> + Send + 'static,
    {
        self.callback = Some(callback_from(func));
        self
    }

    pub fn with_dont_filter(mut self, dont_filter: bool) -> Self {
        self.dont_filter = dont_filter;
        self
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

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
    pub fn new(url: impl Into<String>) -> Self {
        RequestBuilder {
            request: Request::new(url),
        }
    }

    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.request.headers.insert(key.into(), value.into());
        self
    }

    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.request.params.insert(key.into(), value.into());
        self
    }

    pub fn json(mut self, value: impl Into<Item>) -> Self {
        self.request.json = Some(value.into());
        self
    }

    pub fn data(mut self, data: Vec<u8>) -> Self {
        self.request.data = Some(data);
        self
    }

    pub fn callback_fn<F, Fut>(mut self, func: F) -> Self
    where
        S: Send + Sync + 'static,
        F: Fn(Arc<S>, Response<S>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = SpiderResult<S>> + Send + 'static,
    {
        self.request.callback = Some(callback_from(func));
        self
    }

    pub fn dont_filter(mut self, dont_filter: bool) -> Self {
        self.request.dont_filter = dont_filter;
        self
    }

    pub fn priority(mut self, priority: i32) -> Self {
        self.request.priority = priority;
        self
    }

    pub fn build(self) -> Request<S> {
        self.request
    }
}

#[derive(Clone)]
pub enum SpiderOutput<S> {
    Request(Box<Request<S>>),
    Item(Item),
}

pub type SpiderResult<S> = Vec<SpiderOutput<S>>;

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

pub fn callback_from_fn<S, Fut>(func: fn(Arc<S>, Response<S>) -> Fut) -> Callback<S>
where
    S: Send + Sync + 'static,
    Fut: Future<Output = SpiderResult<S>> + Send + 'static,
{
    callback_from(func)
}

#[cfg(test)]
mod tests {
    use super::{Request, SpiderOutput, callback_from, callback_from_fn};
    use crate::response::Response;
    use crate::types::{Headers, Item};
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
    fn request_builder_sets_fields() {
        let req = Request::<()>::builder("https://example.com/search")
            .header("Accept", "text/html")
            .param("q", "rust")
            .json(Item::from(1))
            .data(vec![1, 2, 3])
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
        assert_eq!(req.data.as_ref().map(Vec::len), Some(3));
        assert!(req.dont_filter);
        assert_eq!(req.priority, 9);
    }

    #[tokio::test]
    async fn request_builder_callback_sets_closure() {
        let req = Request::<TestSpider>::builder("https://example.com")
            .callback_fn(|_spider: Arc<TestSpider>, _response: Response<TestSpider>| async {
                Vec::new()
            })
            .build();

        assert!(req.callback.is_some());
    }

    #[tokio::test]
    async fn callback_from_fn_wraps_function() {
        async fn handler(
            _spider: Arc<TestSpider>,
            _response: Response<TestSpider>,
        ) -> Vec<SpiderOutput<TestSpider>> {
            vec![Item::from("ok").into()]
        }

        let callback = callback_from_fn(handler);
        let request = Request::<TestSpider>::new("https://example.com");
        let response = Response {
            url: "https://example.com".to_string(),
            status: 200,
            headers: Headers::new(),
            body: Vec::new(),
            request,
        };

        let outputs = callback(Arc::new(TestSpider), response).await;
        assert_eq!(outputs.len(), 1);
    }

    #[tokio::test]
    async fn callback_from_accepts_closure() {
        let suffix = "ok".to_string();
        let callback = callback_from(move |_spider: Arc<TestSpider>, _response: Response<TestSpider>| {
            let suffix = suffix.clone();
            async move { vec![Item::from(suffix).into()] }
        });

        let request = Request::<TestSpider>::new("https://example.com");
        let response = Response {
            url: "https://example.com".to_string(),
            status: 200,
            headers: Headers::new(),
            body: Vec::new(),
            request,
        };

        let outputs = callback(Arc::new(TestSpider), response).await;
        assert_eq!(outputs.len(), 1);
    }

    #[tokio::test]
    async fn request_with_callback_fn_accepts_closure() {
        let request = Request::<TestSpider>::new("https://example.com").with_callback_fn(
            |_spider: Arc<TestSpider>, _response: Response<TestSpider>| async {
                vec![Item::from("ok").into()]
            },
        );

        let callback = request.callback.clone().expect("callback");
        let response = Response {
            url: "https://example.com".to_string(),
            status: 200,
            headers: Headers::new(),
            body: Vec::new(),
            request,
        };

        let outputs = callback(Arc::new(TestSpider), response).await;
        assert_eq!(outputs.len(), 1);
    }
}
