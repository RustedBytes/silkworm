use serde::Serialize;
use serde_json::{Number, Value};
use std::sync::Arc;
use std::time::Duration;

use silkworm::{Item, SilkwormResult, crawl_with, item_from, prelude::*};

#[path = "support/mock_server.rs"]
mod mock_server;

const SHORT_QUOTE_MIN_LEN: usize = 10;
const PREVIEW_CHAR_LIMIT: usize = 50;
const USER_AGENT: &str = "silkworm-rs/callback-pipeline-demo";

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
        "quotes_callback"
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

        Ok(out)
    }
}

fn snippet(text: &str, max: usize) -> String {
    let truncated = text.chars().take(max).collect::<String>();
    if text.chars().count() > max {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn item_author(item: &Item) -> Option<&str> {
    item.get("author").and_then(|value| value.as_str())
}

fn item_text(item: &Item) -> Option<&str> {
    item.get("text").and_then(|value| value.as_str())
}

fn item_tag_count(item: &Item) -> usize {
    item.get("tags")
        .and_then(|value| value.as_array())
        .map(std::vec::Vec::len)
        .unwrap_or(0)
}

fn print_item(item: Item, spider: Arc<QuotesSpider>) -> SilkwormResult<Item> {
    let author = item_author(&item).unwrap_or("unknown");
    let text = item_text(&item).unwrap_or("");
    println!(
        "[{}] {}: {}",
        spider.name(),
        author,
        snippet(text, PREVIEW_CHAR_LIMIT)
    );
    Ok(item)
}

async fn validate_item(item: Item, _spider: Arc<QuotesSpider>) -> SilkwormResult<Item> {
    if let Some(text) = item_text(&item)
        && text.trim().chars().count() < SHORT_QUOTE_MIN_LEN
    {
        println!("Warning: short quote found");
    }
    Ok(item)
}

fn enrich_item(mut item: Item, spider: Arc<QuotesSpider>) -> SilkwormResult<Item> {
    let tag_count = item_tag_count(&item);

    if let Some(map) = item.as_object_mut() {
        map.insert(
            "spider".to_string(),
            Value::String(spider.name().to_string()),
        );
        map.insert(
            "tag_count".to_string(),
            Value::Number(Number::from(tag_count as u64)),
        );
    }

    Ok(item)
}

#[tokio::main]
async fn main() -> silkworm::SilkwormResult<()> {
    let mock = mock_server::maybe_start().await?;
    let start_url = mock
        .as_ref()
        .map(|server| server.quotes_page_url(1))
        .unwrap_or_else(|| "https://quotes.toscrape.com/page/1/".to_string());

    let config = RunConfig::new()
        .with_concurrency(4)
        .with_request_middleware(UserAgentMiddleware::new(
            vec![],
            Some(USER_AGENT.to_string()),
        ))
        .with_response_middleware(RetryMiddleware::new(3, None, None, 0.5))
        .with_item_pipeline(CallbackPipeline::new(validate_item))
        .with_item_pipeline(CallbackPipeline::from_sync(enrich_item))
        .with_item_pipeline(CallbackPipeline::from_sync(print_item))
        .with_request_timeout(Duration::from_secs(10))
        .with_log_stats_interval(Duration::from_secs(10))
        .with_fail_fast(true);
    crawl_with(QuotesSpider { start_url }, config).await
}
