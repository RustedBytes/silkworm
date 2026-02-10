# Configuration and Runtime

Silkworm exposes `RunConfig` as the primary configuration surface for crawls.
`RunConfig` is converted into `EngineConfig` internally.

## RunConfig

Key settings:
- `concurrency`: number of worker tasks and HTTP concurrency.
- `request_timeout`: per-request timeout override.
- `log_stats_interval`: optional periodic stats logging.
- `max_pending_requests`: size of the request queue (must be greater than 0
  when set).
- `max_seen_requests`: cap for the in-memory de-duplication set (defaults to
  `100_000`).
- `with_unbounded_seen_requests()`: disables the cap for long-running crawls
  where full history is required.
- `html_max_size_bytes`: maximum response body bytes buffered by the HTTP
  client and parsed into `HtmlResponse`.
- `keep_alive`: adds `Connection: keep-alive` header when missing.
- `request_middlewares`, `response_middlewares`, `item_pipelines`.

Code:
- RunConfig: `../src/runner.rs`

```rust
use std::time::Duration;
use silkworm::{DelayMiddleware, JsonLinesPipeline, RunConfig, UserAgentMiddleware};

let config = RunConfig::new()
    .with_concurrency(32)
    .with_max_seen_requests(50_000)
    .with_request_timeout(Duration::from_secs(10))
    .with_request_middleware(UserAgentMiddleware::new(
        vec![],
        Some("silkworm-rs/docs-example".to_string()),
    ))
    .with_request_middleware(DelayMiddleware::fixed(0.2))
    .with_item_pipeline(JsonLinesPipeline::new("data/items.jl"));
```

## EngineConfig

`EngineConfig` mirrors the fields from `RunConfig` and is used to initialize
`Engine` and its HTTP client.

Code:
- EngineConfig: `../src/engine.rs`

## Runtime Entry Points

- `crawl` / `crawl_with`: async APIs that run on an existing Tokio runtime.
- `run_spider` / `run_spider_with`: synchronous helpers that create a runtime.

Note: `run_spider` and `run_spider_with` return a config error when called
inside an existing Tokio runtime. Use `crawl`/`crawl_with` in async contexts.

Code:
- Run helpers: `../src/runner.rs`

```rust
// Async usage
silkworm::crawl(QuotesSpider).await?;

// Sync usage (spawns its own Tokio runtime)
silkworm::run_spider(QuotesSpider)?;
```

## HtmlResponse Limits

`html_max_size_bytes` limits two stages:

- HTTP body buffering in `HttpClient`.
- HTML decoding/parsing when converting `Response` into `HtmlResponse`.

If the response body exceeds the limit, only the initial slice is retained.

Code:
- HtmlResponse creation: `../src/response.rs`
- Engine response handling: `../src/engine.rs`

```rust
let config = RunConfig::new().with_html_max_size_bytes(2_000_000);
```
