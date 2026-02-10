use serde::Serialize;
use std::time::Duration;

use silkworm::{crawl_with, prelude::*};

#[path = "support/mock_server.rs"]
mod mock_server;

const USER_AGENT: &str = "silkworm-rs/quotes-spider";

struct QuotesSpider {
    start_url: String,
}

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

    async fn start_requests(&self) -> Vec<Request<Self>> {
        vec![Request::get(self.start_url.clone())]
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
    let mock = mock_server::maybe_start().await?;
    let start_url = mock
        .as_ref()
        .map(|server| server.quotes_root_url())
        .unwrap_or_else(|| "https://quotes.toscrape.com/".to_string());

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
    crawl_with(QuotesSpider { start_url }, config).await
}
