use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

use silkworm::{
    run_spider_with, HtmlResponse, JsonLinesPipeline, RetryMiddleware, RunConfig, Spider,
    SpiderResult, UserAgentMiddleware,
};

struct QuotesSpider;

#[async_trait]
impl Spider for QuotesSpider {
    fn name(&self) -> &str {
        "quotes"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://quotes.toscrape.com/".to_string()]
    }

    async fn parse(&self, response: HtmlResponse<Self>) -> SpiderResult<Self> {
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
            let tags = match quote.select(".tag") {
                Ok(nodes) => nodes,
                Err(_) => Vec::new(),
            };
            let tag_values = tags.into_iter().map(|tag| tag.text()).collect::<Vec<_>>();

            out.push(
                json!({
                    "text": text_el.text(),
                    "author": author_el.text(),
                    "tags": tag_values,
                })
                .into(),
            );
        }

        if let Ok(Some(link)) = response.select_first("li.next > a") {
            if let Some(href) = link.attr("href") {
                out.push(response.follow(&href, None).into());
            }
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
    config.item_pipelines = vec![Arc::new(JsonLinesPipeline::new("data/quotes.jl"))];
    config.request_timeout = Some(Duration::from_secs(10));
    config.log_stats_interval = Some(Duration::from_secs(10));
    run_spider_with(QuotesSpider, config)
}
