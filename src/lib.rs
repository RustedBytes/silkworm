#![forbid(unsafe_code)]

pub mod api;
pub mod engine;
pub mod errors;
pub mod http;
pub mod logging;
pub mod middlewares;
pub mod pipelines;
pub mod request;
pub mod response;
pub mod runner;
pub mod spider;
pub mod types;

pub use api::fetch_html;
pub use engine::Engine;
pub use errors::{SilkwormError, SilkwormResult};
pub use http::HttpClient;
pub use logging::{Logger, complete_logs, get_logger};
pub use middlewares::{
    DelayMiddleware, ProxyMiddleware, RequestMiddleware, ResponseAction, ResponseMiddleware,
    RetryMiddleware, SkipNonHtmlMiddleware, UserAgentMiddleware,
};
pub use pipelines::{CallbackPipeline, CsvPipeline, ItemPipeline, JsonLinesPipeline, XmlPipeline};
pub use request::{
    Callback, CallbackFuture, Request, SpiderOutput, SpiderResult, callback_from_fn,
};
pub use response::{HtmlElement, HtmlResponse, Response};
pub use runner::{RunConfig, crawl, crawl_with, run_spider, run_spider_with};
pub use spider::Spider;
pub use types::{Headers, Item, Meta, Params};
