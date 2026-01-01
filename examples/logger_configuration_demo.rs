use async_trait::async_trait;
use serde_json::json;

use silkworm::{get_logger, run_spider, HtmlResponse, Logger, Spider, SpiderResult};

struct LoggingSpider {
    logger: Logger,
}

impl LoggingSpider {
    fn new() -> Self {
        let logger = get_logger("LoggingSpider", Some("logging_demo"))
            .bind("demo", "logger_configuration")
            .bind("env", "local");
        LoggingSpider { logger }
    }
}

#[async_trait]
impl Spider for LoggingSpider {
    fn name(&self) -> &str {
        "logging_demo"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://quotes.toscrape.com/".to_string()]
    }

    async fn parse(&self, response: HtmlResponse<Self>) -> SpiderResult<Self> {
        self.logger.info(
            "Parsing page",
            &[
                ("url", response.url.clone()),
                ("status", response.status.to_string()),
            ],
        );
        vec![json!({"url": response.url, "status": response.status}).into()]
    }
}

fn main() -> silkworm::SilkwormResult<()> {
    let spider = LoggingSpider::new();
    run_spider(spider)
}
