# Silkworm (Rust)

Async-first web scraping framework inspired by Silkworm, built on `reqwest` + `scraper`.
It provides a minimal Spider/Request/Response model, middlewares, pipelines, and
structured logging so you can write crawlers without boilerplate.

## Features

- Async engine with configurable concurrency and bounded backpressure.
- Request/Response model with helpers for redirects, params merging, and `follow`.
- HTML parsing helpers via `scraper` (`select`, `select_first`, `css`, `css_first`).
- Middleware system (User-Agent, proxy, delay, retry, skip non-HTML).
- Pipelines for JSON Lines, CSV, XML, or custom callbacks.
- Structured logging with crawl statistics (`SILKWORM_LOG_LEVEL`).

## Install

```bash
cargo add silkworm
```

If you want to run the async API yourself (instead of `run_spider`), add Tokio:

```bash
cargo add tokio --features rt-multi-thread,macros
```

## Quick Start

```rust
use async_trait::async_trait;
use serde_json::json;
use silkworm::{run_spider, HtmlResponse, Spider, SpiderResult};

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
            out.push(json!({
                "text": text_el.text(),
                "author": author_el.text(),
            }).into());
        }

        out
    }
}

fn main() -> silkworm::SilkwormResult<()> {
    run_spider(QuotesSpider)
}
```

## Pipelines

Write scraped items to files or plug in your own callback:

```rust
use std::sync::Arc;

use silkworm::{run_spider_with, JsonLinesPipeline, RunConfig};

let mut config = RunConfig::default();
config.item_pipelines = vec![Arc::new(JsonLinesPipeline::new("data/items.jl"))];
run_spider_with(QuotesSpider, config)?;
```

Available pipelines:

- `JsonLinesPipeline` (streaming JSON Lines)
- `CsvPipeline` (flattened CSV)
- `XmlPipeline` (nested XML)
- `CallbackPipeline` (custom per-item handler)

## Middlewares

Enable built-ins by adding them to the run config:

```rust
use std::sync::Arc;
use std::time::Duration;

use silkworm::{
    DelayMiddleware, RetryMiddleware, RunConfig, SkipNonHtmlMiddleware, UserAgentMiddleware,
};

let mut config = RunConfig::default();
config.request_middlewares = vec![
    Arc::new(UserAgentMiddleware::new(vec![], Some("silkworm-rs/0.1".to_string()))),
    Arc::new(DelayMiddleware::fixed(0.25)),
];
config.response_middlewares = vec![
    Arc::new(RetryMiddleware::new(3, None, None, 0.5)),
    Arc::new(SkipNonHtmlMiddleware::new(None, 1024)),
];
config.request_timeout = Some(Duration::from_secs(15));
```

## Request Helpers

`Response::follow` carries the current callback by default:

```rust
let next = response.follow("/page/2", None);
```

## Logging

Structured logs include crawl statistics and can be adjusted via environment:

```bash
SILKWORM_LOG_LEVEL=DEBUG cargo run
```

## Utility API

Fetch HTML directly and parse with `scraper`:

```rust
let (text, document) = silkworm::fetch_html("https://example.com").await?;
```

## License

MIT
