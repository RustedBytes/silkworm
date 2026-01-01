use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};

use silkworm::{HtmlResponse, Logger, Spider, SpiderResult, get_logger, run_spider};

struct HybridLoggerSpider {
    logger: Logger,
    pages_seen: AtomicUsize,
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

        let quotes = match response.select(".quote") {
            Ok(nodes) => nodes,
            Err(_) => return out,
        };

        for quote in quotes {
            let text_el = match quote.select_first(".text") {
                Ok(Some(el)) => el,
                _ => continue,
            };
            let author_el = match quote.select_first(".author") {
                Ok(Some(el)) => el,
                _ => continue,
            };

            out.push(
                json!({
                    "text": text_el.text(),
                    "author": author_el.text(),
                })
                .into(),
            );
        }

        if page < 2 {
            if let Ok(Some(link)) = response.select_first("li.next > a") {
                if let Some(href) = link.attr("href") {
                    out.push(response.follow(&href, None).into());
                }
            }
        }

        out
    }
}

fn main() -> silkworm::SilkwormResult<()> {
    println!(
        "Redirect stdout to capture logs: cargo run --example hybrid_logger_demo > data/logs.txt"
    );
    run_spider(HybridLoggerSpider::new())
}
