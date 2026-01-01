use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

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

    pub fn with_method(mut self, method: impl Into<String>) -> Self {
        self.method = method.into();
        self
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.insert(key.into(), value.into());
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

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn with_callback(mut self, callback: Callback<S>) -> Self {
        self.callback = Some(callback);
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

pub fn callback_from_fn<S, Fut>(func: fn(Arc<S>, Response<S>) -> Fut) -> Callback<S>
where
    S: Send + Sync + 'static,
    Fut: Future<Output = SpiderResult<S>> + Send + 'static,
{
    Arc::new(move |spider: Arc<S>, response: Response<S>| {
        let fut = func(spider, response);
        Box::pin(fut)
    })
}

#[cfg(test)]
mod tests {
    use super::{Request, SpiderOutput, callback_from_fn};
    use crate::response::Response;
    use crate::types::{Headers, Item};
    use std::sync::Arc;

    struct TestSpider;

    #[test]
    fn request_builder_sets_fields() {
        let req = Request::<()>::new("https://example.com")
            .with_method("POST")
            .with_header("Accept", "text/html")
            .with_param("q", "1")
            .with_data(vec![1, 2])
            .with_json(Item::from(1))
            .with_meta("trace", Item::from("abc"))
            .with_dont_filter(true)
            .with_priority(5);

        assert_eq!(req.url, "https://example.com");
        assert_eq!(req.method, "POST");
        assert_eq!(
            req.headers.get("Accept").map(String::as_str),
            Some("text/html")
        );
        assert_eq!(req.params.get("q").map(String::as_str), Some("1"));
        assert_eq!(req.data.as_ref().map(Vec::len), Some(2));
        assert_eq!(req.json.as_ref().and_then(|v| v.as_i64()), Some(1));
        assert_eq!(req.meta.get("trace").and_then(|v| v.as_str()), Some("abc"));
        assert!(req.dont_filter);
        assert_eq!(req.priority, 5);
    }

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
}
