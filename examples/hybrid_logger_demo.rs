use serde::Serialize;
use std::sync::atomic::{AtomicUsize, Ordering};

use silkworm::{Logger, crawl_with, get_logger, prelude::*};

#[path = "support/mock_server.rs"]
mod mock_server;

struct HybridLoggerSpider {
    logger: Logger,
    start_url: String,
    pages_seen: AtomicUsize,
}

#[derive(Debug, Serialize)]
struct QuoteItem {
    text: String,
    author: String,
}

impl HybridLoggerSpider {
    fn new(start_url: String) -> Self {
        let logger = get_logger("HybridLogger", Some("hybrid_logger_demo"))
            .bind("mode", "stdout")
            .bind("hint", "redirect_output");
        HybridLoggerSpider {
            logger,
            start_url,
            pages_seen: AtomicUsize::new(0),
        }
    }
}

impl Spider for HybridLoggerSpider {
    fn name(&self) -> &str {
        "hybrid_logger_demo"
    }

    async fn start_requests(&self) -> Vec<Request<Self>> {
        vec![Request::get(self.start_url.clone())]
    }

    async fn parse(&self, response: HtmlResponse<Self>) -> SpiderResult<Self> {
        let page = self.pages_seen.fetch_add(1, Ordering::SeqCst) + 1;
        self.logger.info(
            "Parsing page",
            &[("url", response.url.clone()), ("page", page.to_string())],
        );
        let mut out = Vec::new();

        for quote in response.select_or_empty(".quote") {
            let text = quote.text_from(".text");
            let author = quote.text_from(".author");
            if text.is_empty() || author.is_empty() {
                continue;
            }

            let quote = QuoteItem { text, author };
            if let Ok(item) = item_from(quote) {
                out.push(item.into());
            }
        }

        if page < 2 {
            let next_links = response
                .select_or_empty("li.next > a")
                .into_iter()
                .filter_map(|link| link.attr("href"))
                .collect::<Vec<_>>();
            out.extend(response.follow_urls(next_links).into_iter().map(Into::into));
        }

        Ok(out)
    }
}

#[tokio::main]
async fn main() -> silkworm::SilkwormResult<()> {
    let mock = mock_server::maybe_start().await?;
    let start_url = mock
        .as_ref()
        .map(|server| server.quotes_root_url())
        .unwrap_or_else(|| "https://quotes.toscrape.com/".to_string());
    println!(
        "Redirect stdout to capture logs: cargo run --example hybrid_logger_demo > data/logs.txt"
    );
    let config = RunConfig::new().with_fail_fast(true);
    crawl_with(HybridLoggerSpider::new(start_url), config).await
}
