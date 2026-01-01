use serde::Serialize;
use std::time::Duration;

use silkworm::{prelude::*, run_spider_with};

/// Demonstrates the new ergonomic API for selecting elements.
/// Compare this with the original quotes_spider.rs to see the improvements.
struct QuotesSpider;

#[derive(Debug, Serialize)]
struct QuoteItem {
    text: String,
    author: String,
    tags: Vec<String>,
}

impl Spider for QuotesSpider {
    fn name(&self) -> &str {
        "quotes_ergonomic"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://quotes.toscrape.com/".to_string()]
    }

    async fn parse(&self, response: HtmlResponse<Self>) -> SpiderResult<Self> {
        let mut out = Vec::new();

        // Using the new ergonomic API - no need for match statements!
        for quote in response.select_or_empty(".quote") {
            // text_from() extracts text directly, returns empty string if not found
            let text = quote.text_from(".text");
            let author = quote.text_from(".author");
            
            // Skip empty quotes
            if text.is_empty() || author.is_empty() {
                continue;
            }

            // Collect tags using the ergonomic API
            let tag_values = quote.select_texts(".tag");

            let quote = QuoteItem {
                text,
                author,
                tags: tag_values,
            };
            if let Ok(item) = item_from(quote) {
                out.push(item.into());
            }
        }

        // Using follow_css_outputs() for cleaner pagination
        out.extend(response.follow_css_outputs("li.next > a", "href"));

        out
    }
}

fn main() -> silkworm::SilkwormResult<()> {
    let config = RunConfig::new()
        .with_request_middleware(UserAgentMiddleware::new(
            vec![],
            Some("silkworm-rs/0.1".to_string()),
        ))
        .with_response_middleware(RetryMiddleware::new(3, None, None, 0.5))
        .with_item_pipeline(JsonLinesPipeline::new("data/quotes_ergonomic.jl"))
        .with_request_timeout(Duration::from_secs(10))
        .with_log_stats_interval(Duration::from_secs(10));
    run_spider_with(QuotesSpider, config)
}
