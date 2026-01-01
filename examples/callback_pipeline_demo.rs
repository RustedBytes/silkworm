use async_trait::async_trait;
use serde_json::{Number, Value};
use std::sync::Arc;
use std::time::Duration;

use silkworm::{
    run_spider_with, CallbackPipeline, HtmlResponse, Item, RunConfig, SilkwormResult, Spider,
    SpiderResult, UserAgentMiddleware,
};

struct QuotesSpider;

#[async_trait]
impl Spider for QuotesSpider {
    fn name(&self) -> &str {
        "quotes_callback"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://quotes.toscrape.com/page/1/".to_string()]
    }

    async fn parse(&self, response: HtmlResponse<Self>) -> SpiderResult<Self> {
        let mut out = Vec::new();

        let quotes = match response.select(".quote") {
            Ok(nodes) => nodes,
            Err(_) => return out,
        };

        for quote in quotes {
            let text = match quote.select_first(".text") {
                Ok(Some(el)) => el.text(),
                _ => String::new(),
            };
            let author = match quote.select_first(".author") {
                Ok(Some(el)) => el.text(),
                _ => String::new(),
            };
            let tags = match quote.select(".tag") {
                Ok(nodes) => nodes,
                Err(_) => Vec::new(),
            };
            let tag_values = tags.into_iter().map(|tag| tag.text()).collect::<Vec<_>>();

            let mut item = serde_json::Map::new();
            item.insert("text".to_string(), Value::String(text));
            item.insert("author".to_string(), Value::String(author));
            item.insert(
                "tags".to_string(),
                Value::Array(tag_values.into_iter().map(Value::String).collect()),
            );
            out.push(Value::Object(item).into());
        }

        out
    }
}

fn snippet(text: &str, max: usize) -> String {
    text.chars().take(max).collect::<String>()
}

fn print_item(item: Item, spider: Arc<QuotesSpider>) -> SilkwormResult<Item> {
    let author = item
        .get("author")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    let text = item
        .get("text")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    println!("[{}] {}: {}", spider.name(), author, snippet(text, 50));
    Ok(item)
}

async fn validate_item(item: Item, _spider: Arc<QuotesSpider>) -> SilkwormResult<Item> {
    if let Some(text) = item.get("text").and_then(|value| value.as_str()) {
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
    let mut config = RunConfig::default();
    config.concurrency = 4;
    config.request_middlewares = vec![Arc::new(UserAgentMiddleware::new(vec![], None))];
    config.item_pipelines = vec![
        Arc::new(CallbackPipeline::from_sync(print_item)),
        Arc::new(CallbackPipeline::new(validate_item)),
        Arc::new(CallbackPipeline::from_sync(enrich_item)),
    ];
    config.request_timeout = Some(Duration::from_secs(10));
    run_spider_with(QuotesSpider, config)
}
