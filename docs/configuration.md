# Configuration and Runtime

Silkworm exposes `RunConfig` as the primary configuration surface for crawls.
`RunConfig` is converted into `EngineConfig` internally.

## RunConfig

Defaults from `RunConfig::default()`:

| Setting | Default | Meaning |
| --- | --- | --- |
| `concurrency` | `16` | HTTP workers; must be positive |
| `request_timeout` | `None` | No explicit timeout configured by Silkworm |
| `log_stats_interval` | `None` | Periodic statistics disabled |
| `max_pending_requests` | `None` | Ready/delayed request backlog unbounded |
| `max_seen_requests` | `Some(100_000)` | FIFO history cap; must be positive when set |
| `html_max_size_bytes` | `5_000_000` | Retained response bytes |
| `keep_alive` | `false` | No explicit keep-alive header injection |
| `fail_fast` | `false` | Processing errors logged; crawl continues |
| Middleware/pipeline lists | Empty | Built-ins are opt-in |

Key settings:
- `concurrency`: number of worker tasks and HTTP concurrency.
- `request_timeout`: per-request timeout override.
- `log_stats_interval`: optional periodic stats logging.
- `max_pending_requests`: hard cap for the combined ready and delayed request
  backlog (must be greater than 0 when set). Active workers do not count toward
  this cap. `None` leaves the request backlog unbounded.
- `max_seen_requests`: cap for the in-memory de-duplication set (defaults to
  `100_000`).
- `with_unbounded_seen_requests()`: disables the cap for long-running crawls
  where full history is required.
- `html_max_size_bytes`: maximum response body bytes buffered by the HTTP
  client and parsed into `HtmlResponse`.
- `keep_alive`: adds `Connection: keep-alive` header when missing.
- `fail_fast`: stop the crawl on the first request/parse/pipeline error.
- `request_middlewares`, `response_middlewares`, `item_pipelines`.

Code:
- RunConfig: `../src/runner.rs`

When a configured request cap is exceeded, the crawl returns a `Spider` capacity
error and stops, even with `fail_fast = false`. Workers produce new requests as
well as consume them; making every producer wait for a free slot could deadlock
all workers. The engine therefore reports overload instead of silently dropping
requests or waiting indefinitely. Increase the cap or reduce spider fan-out
when this happens. The cap counts requests, not bytes: active responses,
callback output vectors, item queues and de-duplication history have separate
memory costs. The item channel uses the configured cap as its size, or
`concurrency * 10` when no request cap is configured.

```rust
use std::time::Duration;
use silkworm::{DelayMiddleware, JsonLinesPipeline, RunConfig, UserAgentMiddleware};

let config = RunConfig::<QuotesSpider>::new()
    .with_concurrency(32)
    .with_max_pending_requests(500)
    .with_max_seen_requests(50_000)
    .with_request_timeout(Duration::from_secs(10))
    .with_log_stats_interval(Duration::from_secs(10))
    .with_html_max_size_bytes(2_000_000)
    .with_keep_alive(true)
    .with_fail_fast(true)
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

### Async application

Keep the [README `QuotesSpider` definition](../README.md#quick-start) and replace its `main` with this one.
If you already run a Tokio runtime, use `crawl`/`crawl_with`:

```rust
use silkworm::{crawl_with, RunConfig};

#[tokio::main]
async fn main() -> silkworm::SilkwormResult<()> {
    let config = RunConfig::<QuotesSpider>::new().with_concurrency(32);
    crawl_with(QuotesSpider, config).await
}
```

## Completion and errors

Await `crawl`/`crawl_with`, or let the synchronous helper return, to allow normal
shutdown and pipeline flush. Dropping the crawl future is not an implemented
graceful-shutdown API and does not guarantee that spider hooks or pipeline
`close` run. Keep the Tokio runtime alive while accepted writer commands finish;
see [pipeline cancellation](pipelines.md#file-writer-ownership-and-cancellation).
Startup, capacity and worker-coordination failures remain fatal independently
of `fail_fast`. Processing errors may be logged while a non-fail-fast crawl
returns `Ok(())`; it is not a guarantee that every request or item succeeded.

## HtmlResponse Limits

`html_max_size_bytes` limits two stages:

- HTTP body buffering in `HttpClient`.
- HTML decoding/parsing when converting `Response` into `HtmlResponse`.

If the response body exceeds the limit, only the initial slice is retained.
The HTTP client still drains the rest of the stream, so this cap does not bound
network bytes or total response download time. A body read error after the cap
is still returned. A zero limit retains no body; the resulting HTML can be
empty or incomplete. Truncation is not a selector error.

The utility fetch API has separate defaults: concurrency 8, timeout 15 seconds,
body cap 2,000,000 bytes, redirects enabled with at most 10 redirects and
keep-alive header injection disabled. Engine crawls also follow at most 10
redirects. A request's own timeout takes precedence over the configured timeout.

Code:
- HtmlResponse creation: `../src/response.rs`
- Engine response handling: `../src/engine.rs`

```rust
let config = RunConfig::<QuotesSpider>::new().with_html_max_size_bytes(2_000_000);
```
