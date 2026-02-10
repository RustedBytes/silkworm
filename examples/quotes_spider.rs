use serde::Serialize;
use std::time::Duration;

use silkworm::{crawl_with, prelude::*};

const USER_AGENT: &str = "silkworm-rs/quotes-spider";

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

    fn start_urls(&self) -> Vec<&str> {
        vec!["https://quotes.toscrape.com/"]
    }

    async fn parse(&self, response: HtmlResponse<Self>) -> SpiderResult<Self> {
        let mut out = Vec::new();

        for quote in response.select_or_empty(".quote") {
            let text = quote.text_from(".text");
            let author = quote.text_from(".author");
            if text.is_empty() || author.is_empty() {
                continue;
            }

            let quote = QuoteItem {
                text,
                author,
                tags: quote.select_texts(".tag"),
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

        Ok(out)
    }
}

#[tokio::main]
async fn main() -> silkworm::SilkwormResult<()> {
    let config = RunConfig::new()
        .with_request_middleware(UserAgentMiddleware::new(
            vec![],
            Some(USER_AGENT.to_string()),
        ))
        .with_response_middleware(RetryMiddleware::new(3, None, None, 0.5))
        .with_item_pipeline(JsonLinesPipeline::new("data/quotes.jl"))
        .with_request_timeout(Duration::from_secs(10))
        .with_log_stats_interval(Duration::from_secs(10))
        .with_fail_fast(true);
    crawl_with(QuotesSpider, config).await
}
