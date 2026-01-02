# Core Concepts

This document covers the primary user-facing types: Spider, Request, Response,
HtmlResponse, and HtmlElement, plus the output model that lets spiders emit new
requests or scraped items.

## Spider

The `Spider` trait defines the crawler's identity, lifecycle, start URLs, and
parse logic. It uses async methods returning futures and can be customized
without needing boilerplate.

Key methods:
- `name`: label used in logging.
- `start_urls` / `start_requests`: seed requests.
- `parse`: default parse callback for HTML responses.
- `open` / `close`: lifecycle hooks.
- `log`: per-spider structured logger.

Code:
- Spider trait: `../src/spider.rs`

```rust
struct QuotesSpider;

impl Spider for QuotesSpider {
    fn name(&self) -> &str { "quotes" }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://quotes.toscrape.com/".to_string()]
    }

    fn parse(
        &self,
        response: HtmlResponse<Self>,
    ) -> impl std::future::Future<Output = SpiderResult<Self>> + Send + '_ {
        async { Vec::new() }
    }
}
```

## Request

`Request` is the unit of work the engine queues and fetches.

Important fields:
- `url`, `method`, `headers`, `params`
- `data` or `json` payloads
- `timeout` (overrides global timeout)
- `meta` for metadata (used by middlewares like `proxy`, `retry_times`)
- `callback` to override the spider's `parse` method
- `dont_filter` to bypass de-duplication
- `priority` (stored on the request; not used by the engine today)

Convenience APIs:
- `Request::get`, `Request::post`
- Builder helpers: `with_headers`, `with_params`, `with_json`, `with_meta_*`
- `with_callback_fn` to bind async callbacks directly

Code:
- Request type and builder: `../src/request.rs`

```rust
let request = Request::get("https://example.com/search")
    .with_params([("q", "rust"), ("page", "1")])
    .with_header("Accept", "text/html")
    .with_allow_non_html(false);
```

## Response and HtmlResponse

`Response` represents the raw HTTP result. It exposes:

- `text()` and `encoding()` helpers
- status helpers (`status_ok`, `is_redirect`)
- case-insensitive header lookup
- `follow` helpers for building new requests
- `looks_like_html` heuristics

`HtmlResponse` is a wrapper around `Response` that adds CSS and XPath helpers.
It caches decoded HTML and (optionally) parsed document state when the
`scraper-atomic` feature is enabled.

Code:
- Response and HTML wrappers: `../src/response.rs`

```rust
let next = response.follow_url("/page/2");
let titles = response.select_texts("h2.title");
```

## HtmlElement

`HtmlElement` wraps a fragment of HTML with extracted attributes. It provides:

- `html`, `text`, and `attr` accessors
- CSS selection scoped to the fragment
- convenience helpers (`text_from`, `attr_from`, `select_texts`, `select_attrs`)

Code:
- HtmlElement: `../src/response.rs`

```rust
for item in response.select_or_empty(".item") {
    let name = item.text_from(".name");
    let href = item.attr_from("a", "href");
}
```

## Selector APIs

Silkworm offers both error-returning selectors and ergonomic variants:

- CSS: `select`, `select_first`, `css`, `css_first`
- XPath: `xpath`, `xpath_first`
- Ergonomic: `select_or_empty`, `select_first_or_none`, `xpath_or_empty`,
  `xpath_first_or_none`
- Direct extraction: `text_from`, `attr_from`, `select_texts`, `select_attrs`

Code:
- Selector helpers: `../src/response.rs`

```rust
let nodes = response.select_or_empty(".quote");
let first = response.select_first_or_none(".quote");
let links = response.select_attrs("a.next", "href");
```

## Output Model

Spider callbacks return a `SpiderResult`, which is a list of `SpiderOutput`:

- `SpiderOutput::Request` for new requests.
- `SpiderOutput::Item` for data items.

`Item` is a `serde_json::Value` and can be built via `item_from`.

Code:
- Output types: `../src/request.rs`
- Item helpers: `../src/types.rs`

```rust
use serde_json::json;

let mut out = Vec::new();
out.push(Request::get("https://example.com/page/2").into());
out.push(json!({ "title": "Hello" }).into());
```
