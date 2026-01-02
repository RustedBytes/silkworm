use serde::Serialize;
use std::sync::atomic::{AtomicUsize, Ordering};

use silkworm::{Logger, crawl, get_logger, prelude::*};

struct HybridLoggerSpider {
    logger: Logger,
    pages_seen: AtomicUsize,
}

#[derive(Debug, Serialize)]
struct QuoteItem {
    text: String,
    author: String,
}

impl HybridLoggerSpider {
    fn new() -> Self {
        let logger = get_logger("HybridLogger", Some("hybrid_logger_demo"))
            .bind("mode", "stdout")
            .bind("hint", "redirect_output");
        HybridLoggerSpider {
            logger,
            pages_seen: AtomicUsize::new(0),
        }
    }
}

impl Spider for HybridLoggerSpider {
    fn name(&self) -> &str {
        "hybrid_logger_demo"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://quotes.toscrape.com/".to_string()]
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

        out
    }
}

#[tokio::main]
async fn main() -> silkworm::SilkwormResult<()> {
    println!(
        "Redirect stdout to capture logs: cargo run --example hybrid_logger_demo > data/logs.txt"
    );
    crawl(HybridLoggerSpider::new()).await
}
