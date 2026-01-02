use std::future::Future;

use crate::logging::{Logger, get_logger};
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

#[cfg(test)]
mod tests {
    use super::Spider;
    use crate::request::SpiderResult;
    use crate::response::HtmlResponse;

    struct TestSpider;

    impl Spider for TestSpider {
        fn name(&self) -> &str {
            "test"
        }

        fn start_urls(&self) -> Vec<String> {
            vec![
                "https://example.com".to_string(),
                "https://example.com/next".to_string(),
            ]
        }

        async fn parse(&self, _response: HtmlResponse<Self>) -> SpiderResult<Self> {
            Vec::new()
        }
    }

    #[tokio::test]
    async fn start_requests_defaults_to_start_urls() {
        let spider = TestSpider;
        let requests = spider.start_requests().await;
        let urls: Vec<String> = requests.into_iter().map(|req| req.url).collect();
        assert_eq!(
            urls,
            vec!["https://example.com", "https://example.com/next"]
        );
    }
}
