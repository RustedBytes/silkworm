use serde::Serialize;
use std::time::Duration;

use silkworm::{crawl_with, prelude::*};

struct QuotesSpider;

#[derive(Debug, Serialize)]
struct QuoteItem {
    text: String,
    author: String,
    tags: Vec<String>,
}

impl Spider for QuotesSpider {
    fn name(&self) -> &str {
        "quotes"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://quotes.toscrape.com/".to_string()]
    }

    async fn parse(&self, response: HtmlResponse<Self>) -> SpiderResult<Self> {
        let mut out = Vec::new();

        for quote in response.select_or_empty(".quote") {
            let text = quote.text_from(".text");
            let author = quote.text_from(".author");
            if text.is_empty() || author.is_empty() {
                continue;
            }
            let tag_values = quote
                .select_or_empty(".tag")
                .into_iter()
                .map(|tag| tag.text())
                .collect::<Vec<_>>();

            let quote = QuoteItem {
                text,
                author,
                tags: tag_values,
            };
            if let Ok(item) = item_from(quote) {
                out.push(item.into());
            }
        }

        let next_links = response
            .select_or_empty("li.next > a")
            .into_iter()
            .filter_map(|link| link.attr("href"))
            .collect::<Vec<_>>();
        out.extend(response.follow_urls(next_links).into_iter().map(Into::into));

        out
    }
}

#[tokio::main]
async fn main() -> silkworm::SilkwormResult<()> {
    let config = RunConfig::new()
        .with_request_middleware(UserAgentMiddleware::new(
            vec![],
            Some("silkworm-rs/0.1".to_string()),
        ))
        .with_response_middleware(RetryMiddleware::new(3, None, None, 0.5))
        .with_item_pipeline(JsonLinesPipeline::new("data/quotes.jl"))
        .with_request_timeout(Duration::from_secs(10))
        .with_log_stats_interval(Duration::from_secs(10));
    crawl_with(QuotesSpider, config).await
}
