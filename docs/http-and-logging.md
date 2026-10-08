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
- `fetch_html_with` / `fetch_document_with`: same helpers with configurable
  `UtilityFetchOptions`.
- `UtilityFetcher`: reusable helper that keeps one configured HTTP client for
  multiple calls.

`fetch_html` and `fetch_document` use a shared, lazily initialized default
`HttpClient`. `fetch_*_with` builds a client from the provided options for that
call. Use `UtilityFetcher` when you want to reuse custom options across many
requests.
Default safety options are 15-second timeout, redirect following, and a 2 MB
response-body cap.

Code:
- Utility API: `../src/api.rs`

```rust
let (html, doc) = silkworm::fetch_html("https://example.com").await?;
let document = silkworm::fetch_document("https://example.com").await?;
```

```rust
use std::time::Duration;
use silkworm::{UtilityFetchOptions, fetch_html_with};

let options = UtilityFetchOptions::new()
    .with_timeout(Duration::from_secs(8))
    .with_html_max_size_bytes(512_000)
    .with_header("User-Agent", "silkworm-rs/docs-example");
let (html, doc) = fetch_html_with("https://example.com", options).await?;
```

```rust
use std::time::Duration;
use silkworm::{UtilityFetchOptions, UtilityFetcher};

let options = UtilityFetchOptions::new()
    .with_timeout(Duration::from_secs(8))
    .with_header("User-Agent", "silkworm-rs/docs-example");
let fetcher = UtilityFetcher::new(options)?;
let (html, doc) = fetcher.fetch_html("https://example.com").await?;
```

## Header Model Evaluation

The current public header type is still `HashMap<String, String>` for
compatibility and ergonomics, but duplicate response header values are now
normalized into merged comma-separated values.

Evaluation summary:
- Keeping `HashMap<String, String>` avoids a breaking change in `Request`,
  `Response`, and middleware signatures.
- `HeaderMap`-style multi-value storage would improve fidelity for
  `Set-Cookie` and repeated headers, but would require broad API migration and
  conversion costs across the crate.
- Current decision: preserve the public map type for `0.1.x`; revisit a richer
  header model in a future breaking release when API migration can be
  coordinated.

Planned migration path for `0.2`:
1. Introduce a multi-value header type in parallel (for example,
   `HashMap<String, Vec<String>>`) and conversion helpers from/to the current
   map.
2. Add `Response` helpers for repeated headers (for example cookies) while
   keeping existing `header()` compatibility APIs during transition.
3. Migrate middleware and utility APIs to the richer type behind a feature flag
   first, then make it default in the `0.2` breaking release.

## Performance Regression Checks

The benchmark harness supports optional threshold checks for selector/scheduler
regressions:

```bash
SILKWORM_BENCH_CHECK=1 cargo bench --bench core --features xpath
```

CI runs this check in a dedicated nightly/manual benchmark job.

## Logging

Silkworm uses a lightweight structured logger that prints key/value fields. The
log level is controlled by `SILKWORM_LOG_LEVEL` and defaults to INFO.

Code:
- Logger implementation: `../src/logging.rs`
- Engine and middleware logging usage: `../src/engine.rs`, `../src/middlewares.rs`

```bash
SILKWORM_LOG_LEVEL=DEBUG cargo run
```

### URL parameter preparation and measurement

URL preparation borrows keys and values from `Request.params` through a private
`Cow<str>` merge helper instead of cloning each payload. Existing URL query
pairs remain owned so the parsed URL can be mutated safely. Params still replace
all existing values of matching keys, appended params are sorted, canonical
URLs sort all pairs, and fragments are removed. The returned URL is still an
owned `String`; parsing, percent-encoding and output storage allocate as needed.

Run `cargo bench --bench url_params` for URL construction and canonicalization
with 0, 1, 8 and 64 params, Unicode values, duplicate query keys, empty fields
and invalid URL input. Each case measures 20,000 synchronous operations, two
warmups and nine samples. Fixtures are excluded; parsing, merging, serialization,
error construction where applicable and result destruction are included. This
measures URL preparation rather than HTTP or whole-crawl throughput.

For allocation counts, run
`python scripts/measure_seen_allocations.py --benchmark url_params --toolchain 1.92.0`.
The temporary harness imports the production helper and error types, copies the
project lockfile to preserve dependency resolution, and adds `stats_alloc`
0.1.10 only in that temporary package. Timing runs use no counting allocator.
Counts cover Rust System allocator calls/reallocations, deallocations and
requested bytes inside the workload, excluding fixture setup and reporting.
Peak RSS, allocator usable size, networking and foreign allocations are not
measured. The existing seen-cache allocation command remains the default.
