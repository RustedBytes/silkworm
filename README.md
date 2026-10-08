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

Requires Rust **1.92 or newer**, a C/C++ toolchain, CMake and libclang headers.
The package is `silkworm-rs`; imports use `silkworm`.

```bash
cargo new my-spider
cd my-spider
cargo add silkworm-rs
cargo add serde_json
```

See [installation and optional features](docs/installation.md) for XPath,
charset detection, TL parsing and native build setup.

## Quick Start


Save this complete program as `src/main.rs` and run `cargo run`. It crawls the
external quotes demo site and emits items into the engine. With no item
pipeline configured, items are counted but are not saved or printed. See [file pipelines](docs/pipelines.md#save-items-as-json-lines) to retain them.

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

## Documentation

- [Documentation index](docs/README.md)
- [Core concepts, requests and CSS/XPath selectors](docs/core-concepts.md)
- [Runtime entry points and configuration](docs/configuration.md)
- [Middlewares](docs/middlewares.md) and [item pipelines](docs/pipelines.md)
- [HTTP client, utility fetch API and logging](docs/http-and-logging.md)
- [Architecture](docs/architecture.md) and [errors](docs/errors-and-types.md)
- [Runnable examples, benchmarks and development checks](docs/development.md)
- [Criterion benchmark workloads](docs/benchmarks-criterion.md)

Build the API reference for this checkout with `cargo doc --no-deps --open`.
For optional APIs, see [documentation verification](docs/development.md#verify-documentation).

## License

MIT

## Agent skill

Use the [develop-silkworm skill](skills/develop-silkworm/SKILL.md) to build, test
and optimize crawlers with Codex or ChatGPT. It includes API refresh guidance,
reference recipes and an offline-tested crawler template.

## Docker demo

Build with `docker build -t silkworm-parser .` and run with
`docker run --rm --network none silkworm-parser` to parse local fixture pages.
See [container setup](docs/containers.md) for JSONL export and the test image.
