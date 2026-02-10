use serde::Serialize;

use silkworm::{Logger, crawl_with, get_logger, prelude::*};

#[path = "support/mock_server.rs"]
mod mock_server;

struct LoggingSpider {
    logger: Logger,
    start_url: String,
}

#[derive(Debug, Serialize)]
struct PageStatus {
    url: String,
    status: u16,
}

impl LoggingSpider {
    fn new(start_url: String) -> Self {
        let logger = get_logger("LoggingSpider", Some("logging_demo"))
            .bind("demo", "logger_configuration")
            .bind("env", "local");
        LoggingSpider { logger, start_url }
    }
}

impl Spider for LoggingSpider {
    fn name(&self) -> &str {
        "logging_demo"
    }

    async fn start_requests(&self) -> Vec<Request<Self>> {
        vec![Request::get(self.start_url.clone())]
    }

    async fn parse(&self, response: HtmlResponse<Self>) -> SpiderResult<Self> {
        self.logger.info(
            "Parsing page",
            &[
                ("url", response.url.clone()),
                ("status", response.status.to_string()),
            ],
        );
        let item = PageStatus {
            url: response.url.clone(),
            status: response.status,
        };
        if let Ok(item) = item_from(item) {
            Ok(vec![item.into()])
        } else {
            Ok(Vec::new())
        }
    }
}

#[tokio::main]
async fn main() -> silkworm::SilkwormResult<()> {
    let mock = mock_server::maybe_start().await?;
    let start_url = mock
        .as_ref()
        .map(|server| server.quotes_root_url())
        .unwrap_or_else(|| "https://quotes.toscrape.com/".to_string());
    let spider = LoggingSpider::new(start_url);
    let config = RunConfig::new().with_fail_fast(true);
    crawl_with(spider, config).await
}
