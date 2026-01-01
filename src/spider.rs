use async_trait::async_trait;

use crate::logging::{get_logger, Logger};
use crate::request::{Request, SpiderResult};
use crate::response::HtmlResponse;

#[async_trait]
pub trait Spider: Send + Sync + 'static {
    fn name(&self) -> &str {
        "spider"
    }

    fn start_urls(&self) -> Vec<String> {
        Vec::new()
    }

    async fn start_requests(&self) -> Vec<Request<Self>>
    where
        Self: Sized,
    {
        self.start_urls().into_iter().map(Request::new).collect()
    }

    async fn parse(&self, response: HtmlResponse<Self>) -> SpiderResult<Self>
    where
        Self: Sized;

    async fn open(&self) {}

    async fn close(&self) {}

    fn log(&self) -> Logger {
        get_logger("spider", Some(self.name()))
    }
}
