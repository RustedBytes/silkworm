# Silkworm

[![Crates.io Version](https://img.shields.io/crates/v/silkworm-rs)](https://crates.io/crates/silkworm-rs)
[![Tests](https://github.com/RustedBytes/silkworm/actions/workflows/test.yml/badge.svg)](https://github.com/RustedBytes/silkworm/actions/workflows/test.yml)

Async-first web scraping framework for Rust. Built on [`wreq`](https://crates.io/crates/wreq) + [`scraper`](https://crates.io/crates/scraper) with
XPath support via [`sxd-xpath`](https://crates.io/crates/sxd-xpath). It keeps the API small
(Spider/Request/Response), adds middlewares and pipelines, and ships with
structured logging so you can focus on crawling.

## Features

- Async engine with configurable concurrency, queue limits, and request de-dupe
  (method + canonical URL).
- Minimal Spider/Request/Response model with follow helpers and callback
  overrides.
- HTML helpers for CSS and XPath (`select`, `select_first`, `xpath`,
  `xpath_first`) plus ergonomic variants.
- Built-in middlewares for user agents, proxy rotation, delays, retries, and
  skipping non-HTML.
- Pipelines for JSON Lines, CSV, XML, or custom callbacks.
- Structured logging with periodic crawl statistics (`SILKWORM_LOG_LEVEL`).

## Install

```bash
cargo add silkworm-rs
```

If you want to use the async API directly (instead of `run_spider`), add [Tokio](https://crates.io/crates/tokio):

```bash
cargo add tokio --features rt-multi-thread,macros
```

The examples below also use [`serde_json`](https://crates.io/crates/serde_json) for convenience:

```bash
cargo add serde_json
```

Tip: `use silkworm::prelude::*;` for the most common types.

## Quick Start

```rust
use serde_json::json;
use silkworm::{run_spider, HtmlResponse, Spider, SpiderResult};

struct QuotesSpider;

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

            if !text.is_empty() && !author.is_empty() {
                out.push(json!({
                    "text": text,
                    "author": author,
                }).into());
            }
        }

        // Follow pagination links.
        out.extend(response.follow_css_outputs("li.next a", "href"));
        Ok(out)
    }
}

fn main() -> silkworm::SilkwormResult<()> {
    run_spider(QuotesSpider)
}
```

## Async Entry Point

If you already run a Tokio runtime, use `crawl`/`crawl_with`:

```rust
use silkworm::{crawl_with, RunConfig};

#[tokio::main]
async fn main() -> silkworm::SilkwormResult<()> {
    let config = RunConfig::new().with_concurrency(32);
    crawl_with(QuotesSpider, config).await
}
```

## Pipelines

Write scraped items to files or plug in your own callback:

```rust
use silkworm::{run_spider_with, JsonLinesPipeline, RunConfig};

let config = RunConfig::new().with_item_pipeline(JsonLinesPipeline::new("data/items.jl"));
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
use std::time::Duration;

use silkworm::{
    DelayMiddleware, ProxyMiddleware, RetryMiddleware, RunConfig, SkipNonHtmlMiddleware,
    UserAgentMiddleware,
};

let config = RunConfig::new()
    .with_request_middleware(UserAgentMiddleware::new(
        vec![],
        Some("silkworm-rs/0.1".to_string()),
    ))
    .with_request_middleware(DelayMiddleware::fixed(0.25))
    .with_request_middleware(ProxyMiddleware::new(
        vec!["http://proxy.local:8080".to_string()],
        true,
    ))
    .with_response_middleware(RetryMiddleware::new(3, None, None, 0.5))
    .with_response_middleware(SkipNonHtmlMiddleware::new(None, 1024))
    .with_request_timeout(Duration::from_secs(15));
```

## Request Helpers

`Response::follow_url` carries the current callback by default:

```rust
let next = response.follow_url("/page/2");
```

You can also build new requests fluently:

```rust
use silkworm::Request;

let request = Request::get("https://example.com/search")
    .with_params([("q", "rust"), ("page", "1")])
    .with_headers([("Accept", "text/html"), ("User-Agent", "silkworm-rs/0.1")]);
```

If `SkipNonHtmlMiddleware` is enabled, mark requests you want to handle as JSON/XML:

```rust
let request = Request::get("https://example.com/api")
    .with_headers([("Accept", "application/json")])
    .with_allow_non_html(true);
```

For per-request parsing, attach a callback with `with_callback_fn` or
`callback_from_fn` (signature: `Fn(Arc<S>, Response<S>) -> SpiderResult<S>`).
`SpiderResult<S>` is `Result<Vec<SpiderOutput<S>>, SilkwormError>`.

## Ergonomic Selectors

Silkworm provides both error-returning and convenience selector methods for maximum flexibility:

```rust
// Error-returning methods (when you need precise error handling)
let elements = response.select(".item")?;
let element = response.select_first(".item")?;

// Ergonomic methods (for cleaner code when errors can be ignored)
let elements = response.select_or_empty(".item");  // Returns Vec, never errors
let element = response.select_first_or_none(".item");  // Returns Option

// Direct text/attribute extraction
let title = response.text_from("h1");  // Empty string if not found
let href = response.attr_from("a", "href");  // None if not found
let tags = response.select_texts(".tag");  // Vec<String>
let links = response.select_attrs("a.next", "href");  // Vec<String>

// Follow links directly from selectors
let next_requests = response.follow_css("a.next", "href");

// Also works on HtmlElement for nested selections
for item in response.select_or_empty(".item") {
    let name = item.text_from(".name");
    let price = item.text_from(".price");
}
```

All these methods work with CSS selectors and XPath (use `xpath_or_empty()`,
`xpath_first_or_none()`, etc.).

## Configuration

`RunConfig` controls concurrency, queue sizing, and HTTP behavior:

```rust
use std::time::Duration;
use silkworm::RunConfig;

let config = RunConfig::new()
    .with_concurrency(32)
    .with_max_pending_requests(500)
    .with_max_seen_requests(50_000)
    .with_log_stats_interval(Duration::from_secs(10))
    .with_request_timeout(Duration::from_secs(10))
    .with_html_max_size_bytes(2_000_000)
    .with_keep_alive(true);
```

`max_pending_requests` must be greater than `0` when set.  
`html_max_size_bytes` also bounds how many bytes the HTTP client buffers per response.

## Logging

Structured logs include crawl statistics and can be adjusted via environment:

```bash
SILKWORM_LOG_LEVEL=DEBUG cargo run
```

## Utility API

Fetch HTML directly and parse with [`scraper`](https://crates.io/crates/scraper):

```rust
#[tokio::main]
async fn main() -> silkworm::SilkwormResult<()> {
    let (text, document) = silkworm::fetch_html("https://example.com").await?;
    Ok(())
}
```

If you only need a parsed document:

```rust
#[tokio::main]
async fn main() -> silkworm::SilkwormResult<()> {
    let document = silkworm::fetch_document("https://example.com").await?;
    Ok(())
}
```

## Examples

Check out the runnable examples in `examples/`, including:

- `examples/quotes_spider.rs`
- `examples/quotes_spider_xpath.rs`
- `examples/hackernews_spider.rs`
- `examples/sitemap_spider.rs`

## Benchmarks

Run the built-in benchmark suite:

```bash
cargo bench --bench core --features="xpath"
```

The suite measures request construction/cloning, response decoding, URL follow
helpers, and CSS/XPath extraction (including parse-each-time vs cached paths).

## License

MIT
