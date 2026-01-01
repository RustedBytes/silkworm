use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use std::sync::Arc;
use std::time::Duration;

use silkworm::{Item, SilkwormResult, prelude::*, run_spider_with};

struct QuotesSpider;

#[derive(Debug, Serialize, Deserialize)]
struct QuoteItem {
    text: String,
    author: String,
    tags: Vec<String>,
}

impl Spider for QuotesSpider {
    fn name(&self) -> &str {
        "quotes_callback"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://quotes.toscrape.com/page/1/".to_string()]
    }

    async fn parse(&self, response: HtmlResponse<Self>) -> SpiderResult<Self> {
        let mut out = Vec::new();

        for quote in response.select_or_empty(".quote") {
            let text = quote.text_from(".text");
            let author = quote.text_from(".author");
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

        out
    }
}

fn snippet(text: &str, max: usize) -> String {
    text.chars().take(max).collect::<String>()
}

fn print_item(item: Item, spider: Arc<QuotesSpider>) -> SilkwormResult<Item> {
    let parsed = item_into::<QuoteItem>(item.clone()).ok();
    let (author, text) = parsed
        .map(|quote| (quote.author, quote.text))
        .unwrap_or_else(|| {
            let author = item
                .get("author")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown")
                .to_string();
            let text = item
                .get("text")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string();
            (author, text)
        });
    println!("[{}] {}: {}", spider.name(), author, snippet(text, 50));
    Ok(item)
}

async fn validate_item(item: Item, _spider: Arc<QuotesSpider>) -> SilkwormResult<Item> {
    let text = item_into::<QuoteItem>(item.clone())
        .ok()
        .map(|quote| quote.text)
        .or_else(|| item.get("text").and_then(|value| value.as_str()).map(str::to_string));

    if let Some(text) = text {
        if text.trim().len() < 10 {
            println!("Warning: short quote found");
        }
    }
    Ok(item)
}

fn enrich_item(mut item: Item, spider: Arc<QuotesSpider>) -> SilkwormResult<Item> {
    let tag_count = item
        .get("tags")
        .and_then(|value| value.as_array())
        .map(|values| values.len())
        .unwrap_or(0);

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

fn main() -> silkworm::SilkwormResult<()> {
    let config = RunConfig::new()
        .with_concurrency(4)
        .with_request_middleware(UserAgentMiddleware::new(vec![], None))
        .with_item_pipeline(CallbackPipeline::from_sync(print_item))
        .with_item_pipeline(CallbackPipeline::new(validate_item))
        .with_item_pipeline(CallbackPipeline::from_sync(enrich_item))
        .with_request_timeout(Duration::from_secs(10));
    run_spider_with(QuotesSpider, config)
}
