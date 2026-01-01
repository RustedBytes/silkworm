use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

use silkworm::{
    HtmlResponse, JsonLinesPipeline, RetryMiddleware, RunConfig, Spider, SpiderResult,
    UserAgentMiddleware, run_spider_with,
};

/// Demonstrates the new ergonomic API for selecting elements.
/// Compare this with the original quotes_spider.rs to see the improvements.
struct QuotesSpider;

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
            let tag_values = quote
                .select_or_empty(".tag")
                .into_iter()
                .map(|tag| tag.text())
                .collect::<Vec<_>>();

            out.push(
                json!({
                    "text": text,
                    "author": author,
                    "tags": tag_values,
                })
                .into(),
            );
        }

        // Using select_first_or_none() and attr_from() for cleaner pagination
        if let Some(href) = response.attr_from("li.next > a", "href") {
            out.push(response.follow(&href, None).into());
        }

        out
    }
}

fn main() -> silkworm::SilkwormResult<()> {
    let mut config = RunConfig::default();
    config.request_middlewares = vec![Arc::new(UserAgentMiddleware::new(
        vec![],
        Some("silkworm-rs/0.1".to_string()),
    ))];
    config.response_middlewares = vec![Arc::new(RetryMiddleware::new(3, None, None, 0.5))];
    config.item_pipelines = vec![Arc::new(JsonLinesPipeline::new("data/quotes_ergonomic.jl"))];
    config.request_timeout = Some(Duration::from_secs(10));
    config.log_stats_interval = Some(Duration::from_secs(10));
    run_spider_with(QuotesSpider, config)
}
