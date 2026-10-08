# Core Concepts

This document covers the primary user-facing types: Spider, Request, Response,
HtmlResponse, and HtmlElement, plus the output model that lets spiders emit new
requests or scraped items.

## Spider

The `Spider` trait defines the crawler's identity, lifecycle, start URLs, and
parse logic. It uses async methods and can be customized without needing
boilerplate.

Key methods:
- `name`: label used in logging.
- `start_urls` / `start_requests`: seed requests.
- `parse`: default parse callback for HTML responses.
- `open` / `close`: lifecycle hooks.
- `log`: per-spider structured logger.

Code:
- Spider trait: `../src/spider.rs`

```rust
use silkworm::{HtmlResponse, Spider, SpiderResult};

struct QuotesSpider;

impl Spider for QuotesSpider {
    fn name(&self) -> &str { "quotes" }

    fn start_urls(&self) -> Vec<&str> {
        vec!["https://quotes.toscrape.com/"]
    }

    async fn parse(&self, _response: HtmlResponse<Self>) -> SpiderResult<Self> {
        Ok(Vec::new())
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
- `dont_filter` to bypass de-duplication (method + canonical URL)
- `priority` (higher values are scheduled first; FIFO is preserved among equal priorities)

Convenience APIs:
- `Request::get`, `Request::post`
- Builder helpers: `with_headers`, `with_params`, `with_json`, `with_meta_*`
- `with_callback_fn` to bind async callbacks directly
- `request::meta_keys::*` constants for middleware-related metadata keys
- Typed metadata helpers: `meta_str`, `meta_bool`, `meta_u64`, `meta_f64`,
  plus contract helpers such as `proxy`, `retry_times`, and
  `take_retry_delay_secs`

Code:
- Request type and builder: `../src/request.rs`

```rust
use silkworm::Request;

let request = Request::<QuotesSpider>::get("https://example.com/search")
    .with_params([("q", "rust"), ("page", "1")])
    .with_header("Accept", "text/html")
    .with_allow_non_html(false);
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

## Response and HtmlResponse

`Response` represents the raw HTTP result. It exposes:

- `text()` and `encoding()` helpers
- status helpers (`status_ok`, `is_redirect`)
- case-insensitive header lookup
- `follow` helpers for building new requests
- `looks_like_html` heuristics

Decoding uses BOM, then supported HTTP charset, then HTML meta/XML declarations.
Without a declaration, valid UTF-8 is decoded directly. Enabling the optional
`charset-detection` feature (Rust >= 1.98) invokes `charset-norm` for remaining
non-UTF-8 payloads. The detector's best candidate supplies both the text and encoding;
known encodings retain the existing encoding_rs label spelling. Detection is heuristic:
short or ambiguous payloads may be misidentified. A rejected/no-match payload keeps
the existing lossy UTF-8 fallback. Without this feature the fallback is unchanged.
The existing bounded decode cache is shared by `text()` and `encoding()`, and HTML
selection uses the same decoder. No request configuration or public signature changes.

`HtmlResponse` is a wrapper around `Response` that adds CSS helpers and optional
XPath evaluation.
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

CSS extraction helpers do not evaluate XPath. `xpath`/`xpath_first` return a
`Selector` error without the `xpath` feature or for invalid expressions;
convenience variants suppress errors into empty results. `HtmlElement` has CSS
selection only. Prefer error-returning APIs when an invalid selector should
stop parsing. A valid selector with no matches is not an error.

## Output Model

Spider callbacks return a `SpiderResult`, which is:

- `Result<Vec<SpiderOutput<S>>, SilkwormError>`

- `SpiderOutput::Request` for new requests.
- `SpiderOutput::Item` for data items.

`Item` is a `serde_json::Value` and can be built via `item_from`.

Code:
- Output types: `../src/request.rs`
- Item helpers: `../src/types.rs`

```rust
use serde_json::json;

let mut out = Vec::new();
out.push(Request::<QuotesSpider>::get("https://example.com/page/2").into());
out.push(json!({ "title": "Hello" }).into());
Ok(out)
```
