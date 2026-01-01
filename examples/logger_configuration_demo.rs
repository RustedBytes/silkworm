use serde::Serialize;

use silkworm::{Logger, get_logger, prelude::*, run_spider};

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
        let item = PageStatus {
            url: response.url,
            status: response.status,
        };
        if let Ok(item) = item_from(item) {
            vec![item.into()]
        } else {
            Vec::new()
        }
    }
}

fn main() -> silkworm::SilkwormResult<()> {
    let spider = LoggingSpider::new();
    run_spider(spider)
}
