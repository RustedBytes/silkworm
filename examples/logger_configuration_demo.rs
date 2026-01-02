use serde::Serialize;

use silkworm::{Logger, crawl, get_logger, prelude::*};

struct LoggingSpider {
    logger: Logger,
}

#[derive(Debug, Serialize)]
struct PageStatus {
    url: String,
    status: u16,
}

impl LoggingSpider {
    fn new() -> Self {
        let logger = get_logger("LoggingSpider", Some("logging_demo"))
            .bind("demo", "logger_configuration")
            .bind("env", "local");
        LoggingSpider { logger }
    }
}

impl Spider for LoggingSpider {
    fn name(&self) -> &str {
        "logging_demo"
    }

    fn start_urls(&self) -> Vec<&str> {
        vec!["https://quotes.toscrape.com/"]
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
            vec![item.into()]
        } else {
            Vec::new()
        }
    }
}

#[tokio::main]
async fn main() -> silkworm::SilkwormResult<()> {
    let spider = LoggingSpider::new();
    crawl(spider).await
}
