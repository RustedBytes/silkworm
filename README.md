# Silkworm

[![Crates.io Version](https://img.shields.io/crates/v/silkworm-rs)](https://crates.io/crates/silkworm-rs)
[![Tests](https://github.com/RustedBytes/silkworm/actions/workflows/test.yml/badge.svg)](https://github.com/RustedBytes/silkworm/actions/workflows/test.yml)

Async-first web scraping framework for Rust. Built on [`wreq`](https://crates.io/crates/wreq) + [`scraper`](https://crates.io/crates/scraper) with
Optional XPath support via [`xee-xpath`](https://crates.io/crates/xee-xpath). It keeps the API small
(Spider/Request/Response), adds middlewares and pipelines, and ships with
structured logging so you can focus on crawling.

## Features

- Async engine with configurable concurrency, queue limits, and request de-dupe
  (method + canonical URL), plus priority-aware scheduling (`Request.priority`).
- Optional `fail_fast` mode to stop on first processing error.
- Minimal Spider/Request/Response model with follow helpers and callback
  overrides.
- HTML helpers for CSS and XPath (`select`, `select_first`, `xpath`,
  `xpath_first`) plus ergonomic variants.
- Built-in middlewares for user agents, proxy rotation, delays, retries, and
  skipping non-HTML.
- Pipelines for JSON Lines, CSV, XML, or custom callbacks.
- Structured logging with periodic crawl statistics (`SILKWORM_LOG_LEVEL`).

## Install

The current repository requires Rust **1.92 or newer** (edition 2024).
The package is `silkworm-rs`; Rust imports use `silkworm`.
The commands below install the published crate. To use the current checkout,
see [development and verification](docs/development.md).

Create a binary project with `cargo new my-spider`, then run these commands
inside it. Native TLS dependencies require a C/C++ toolchain, CMake and libclang
with its headers available to bindgen (see the development guide).

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

Enable XPath explicitly when needed:

```bash
cargo add silkworm-rs --features xpath
```

The default `scraper-atomic` feature caches parsed HTML. Without default
features, CSS remains available but documents are parsed per selection.
`cli-examples` enables clap-based repository examples; it is not required by
application spiders.

Enable automatic detection of undeclared non-UTF-8 text with
`cargo add silkworm-rs --features charset-detection`. This optional feature uses
[`charset-norm`](https://docs.rs/charset-norm/) and requires **Rust 1.98 or newer**;
default builds retain Rust 1.92 support. BOM, HTTP charset and HTML/XML declarations
remain authoritative, and valid UTF-8 bypasses statistical detection. See
[response decoding](docs/core-concepts.md#response-and-htmlresponse).

Tip: `use silkworm::prelude::*;` for the most common types.

## Quick Start

Save this complete program as `src/main.rs` and run `cargo run`. It crawls the
external quotes demo site and emits items into the engine. With no item
pipeline configured, items are counted but are not saved or printed. Add the
JSON Lines pipeline below to retain them.

Except for complete programs with `main`, the snippets below are fragments to
use with this `QuotesSpider` definition. Selector fragments belong inside
`parse`, where `response: HtmlResponse<Self>` is available.

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

Keep the `QuotesSpider` definition above and replace its `main` with this one.
If you already run a Tokio runtime, use `crawl`/`crawl_with`:

```rust
use silkworm::{crawl_with, RunConfig};

#[tokio::main]
async fn main() -> silkworm::SilkwormResult<()> {
    let config = RunConfig::<QuotesSpider>::new().with_concurrency(32);
    crawl_with(QuotesSpider, config).await
}
```

## Pipelines

Write scraped items to files or plug in your own callback:

```rust
use silkworm::{run_spider_with, JsonLinesPipeline, RunConfig};

let config = RunConfig::<QuotesSpider>::new().with_item_pipeline(JsonLinesPipeline::new("data/items.jl"));
run_spider_with(QuotesSpider, config)?;
```

The pipeline fragment replaces the body of the synchronous `main` (return
`Ok(())` after the call). JSON Lines appends to an existing file.

Available pipelines:

- `JsonLinesPipeline` (streaming JSON Lines)
- `CsvPipeline` (flattened CSV)
- `XmlPipeline` (nested XML)
- `CallbackPipeline` (custom per-item handler)

## Middlewares

Enable built-ins by adding them to the run config. The proxy address below is
a placeholder; supply a working proxy or omit that middleware:

```rust
use std::time::Duration;

use silkworm::{
    DelayMiddleware, ProxyMiddleware, RetryMiddleware, RunConfig, SkipNonHtmlMiddleware,
    UserAgentMiddleware,
};

let config = RunConfig::<QuotesSpider>::new()
    .with_request_middleware(UserAgentMiddleware::new(
        vec![],
        Some("silkworm-rs/example-spider".to_string()),
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

let request = Request::<QuotesSpider>::get("https://example.com/search")
    .with_params([("q", "rust"), ("page", "1")])
    .with_headers([
        ("Accept", "text/html"),
        ("User-Agent", "silkworm-rs/example-spider"),
    ]);
```

If `SkipNonHtmlMiddleware` is enabled, mark requests you want to handle as JSON/XML:

```rust
let request = Request::<QuotesSpider>::get("https://example.com/api")
    .with_headers([("Accept", "application/json")])
    .with_allow_non_html(true);
```

Typed metadata accessors are available when middleware contracts need metadata:

```rust
let mut request = Request::<QuotesSpider>::get("https://example.com")
    .with_proxy("http://proxy.local:8080");
let proxy = request.proxy();
let retries = request.retry_times();
request.set_retry_delay_secs(0.5);
```

For per-request parsing, attach a callback with `with_callback_fn` or
`callback_from_fn`. The callback returns a `Send` future whose output is
`SpiderResult<S>` and receives `(Arc<S>, Response<S>)`.
`with_callback_fn` accepts a closure; `callback_from_fn` takes a function pointer.
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

`select*`, `text_from`, `attr_from` and `follow_css*` take CSS selectors.
`HtmlResponse::xpath` and `xpath_first` take XPath expressions and return a
`Selector` error when the `xpath` feature is disabled. Their convenience
variants `xpath_or_empty` and `xpath_first_or_none` swallow that error.
`HtmlElement` provides CSS helpers; it does not provide XPath methods.

## Configuration

`RunConfig` controls concurrency, queue sizing, and HTTP behavior:

```rust
use std::time::Duration;
use silkworm::RunConfig;

let config = RunConfig::<QuotesSpider>::new()
    .with_concurrency(32)
    .with_max_pending_requests(500)
    .with_max_seen_requests(50_000)
    .with_fail_fast(true)
    .with_log_stats_interval(Duration::from_secs(10))
    .with_request_timeout(Duration::from_secs(10))
    .with_html_max_size_bytes(2_000_000)
    .with_keep_alive(true);
```

`max_pending_requests` bounds ready and delayed requests, excluding active
workers. Exceeding it stops the crawl even with `fail_fast = false`. It must
be greater than `0` when set. See [defaults and limits](docs/configuration.md).
`max_seen_requests` defaults to `100_000` to bound dedupe memory usage.
Use `with_unbounded_seen_requests()` if you explicitly want no cap.
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

Utility fetch helpers use safe defaults: 15s timeout, redirect following, and
a 2 MB response-body cap.

Use `UtilityFetchOptions` for per-call configuration:

```rust
use std::time::Duration;
use silkworm::{UtilityFetchOptions, fetch_html_with};

#[tokio::main]
async fn main() -> silkworm::SilkwormResult<()> {
    let options = UtilityFetchOptions::new()
        .with_timeout(Duration::from_secs(8))
        .with_html_max_size_bytes(512_000)
        .with_header("User-Agent", "silkworm-rs/readme-example");
    let (text, document) = fetch_html_with("https://example.com", options).await?;
    Ok(())
}
```

For repeated calls with the same custom options, build a reusable
`UtilityFetcher` once:

```rust
use std::time::Duration;
use silkworm::{UtilityFetchOptions, UtilityFetcher};

#[tokio::main]
async fn main() -> silkworm::SilkwormResult<()> {
    let options = UtilityFetchOptions::new()
        .with_timeout(Duration::from_secs(8))
        .with_header("User-Agent", "silkworm-rs/readme-example");
    let fetcher = UtilityFetcher::new(options)?;
    let (text, document) = fetcher.fetch_html("https://example.com").await?;
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

Check out the runnable [examples](examples/), including:

- `examples/quotes_spider.rs`
- `examples/quotes_spider_xpath.rs`
- `examples/hackernews_spider.rs`
- `examples/sitemap_spider.rs`

Run all examples in offline mode (local mock server, CI-friendly):

```bash
SILKWORM_EXAMPLE_OFFLINE=1 ./scripts/run_examples_offline.sh
```

When `SILKWORM_EXAMPLE_OFFLINE=1` is set, examples use local deterministic
HTML/XML fixtures instead of external websites.

## Benchmarks

For statistical public-API benchmarks with Criterion, run
`cargo bench -p silkworm-rs --bench criterion_api` (optionally add
`--features xpath`). See [Criterion workloads and reproducible comparisons](docs/benchmarks-criterion.md).

Run the built-in benchmark suite:

```bash
cargo bench --bench core --features="xpath"
```

The suite measures request construction/cloning, response decoding, URL follow
helpers, and CSS/XPath extraction (including parse-each-time vs cached paths).

To run regression threshold checks (selectors + scheduler) locally:

```bash
SILKWORM_BENCH_CHECK=1 cargo bench --bench core --features="xpath"
```

Further microbenchmarks measure item accounting, deduplication storage and URL
parameter preparation. See [architecture measurements](docs/architecture.md)
and [URL/allocation measurements](docs/http-and-logging.md).

## Documentation

Start with the [documentation index](docs/README.md). Build the API reference
for the current checkout with `cargo doc --no-deps --all-features --open`;
a published docs.rs build may describe a different revision.

## License

MIT
