# HTTP Client, Utility API, and Logging

This document covers the HTTP layer, convenience fetch helpers, and structured
logging output.

## HttpClient

Silkworm's `HttpClient` wraps `wreq` and enforces concurrency with a semaphore.
It also supports proxy routing and redirect handling.

Key behaviors:
- Merges request headers with configured defaults.
- Supports JSON and raw body payloads.
- Merges query params into the URL.
- Optional keep-alive header injection.
- Redirect handling with a max redirect limit and loop detection.
- Proxy-specific client caching keyed by proxy URL.

Code:
- HttpClient: `../src/http.rs`

```rust
let request = Request::get("https://example.com/search")
    .with_params([("q", "rust"), ("page", "1")])
    .with_header("Accept", "text/html");
```

## Utility Fetch API

The `api` module exposes convenience functions for fetching and parsing HTML
outside of the spider engine:

- `fetch_html`: returns the raw HTML text and parsed document.
- `fetch_document`: returns only the parsed document.

These functions use a shared, lazily initialized `wreq::Client`.

Code:
- Utility API: `../src/api.rs`

```rust
let (html, doc) = silkworm::fetch_html("https://example.com").await?;
let document = silkworm::fetch_document("https://example.com").await?;
```

## Logging

Silkworm uses a lightweight structured logger that prints key/value fields. The
log level is controlled by `SILKWORM_LOG_LEVEL` and defaults to INFO.

Code:
- Logger implementation: `../src/logging.rs`
- Engine and middleware logging usage: `../src/engine.rs`, `../src/middlewares.rs`

```bash
SILKWORM_LOG_LEVEL=DEBUG cargo run
```
