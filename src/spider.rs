use std::future::Future;

use crate::logging::{get_logger, Logger};
use crate::request::{Request, SpiderResult};
use crate::response::HtmlResponse;

pub trait Spider: Send + Sync + 'static {
    fn name(&self) -> &str {
        "spider"
    }

    fn start_urls(&self) -> Vec<String> {
        Vec::new()
    }

    fn start_requests(&self) -> impl Future<Output = Vec<Request<Self>>> + Send + '_
    where
        Self: Sized,
    {
        async move { self.start_urls().into_iter().map(Request::new).collect() }
    }

    fn parse(
        &self,
        response: HtmlResponse<Self>,
    ) -> impl Future<Output = SpiderResult<Self>> + Send + '_
    where
        Self: Sized;

    fn open(&self) -> impl Future<Output = ()> + Send + '_ {
        async {}
    }

    fn close(&self) -> impl Future<Output = ()> + Send + '_ {
        async {}
    }

    fn log(&self) -> Logger {
        get_logger("spider", Some(self.name()))
    }
}
