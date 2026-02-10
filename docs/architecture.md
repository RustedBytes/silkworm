# Architecture and Data Flow

Silkworm centers on a small asynchronous engine that coordinates a Spider,
request queue, HTTP client, middleware stack, and item pipelines. The engine
owns the lifecycle and is responsible for concurrency, backpressure, and
request de-duplication.

## High-Level Flow

1. The spider is opened and its start requests are generated.
2. Requests are de-duplicated and pushed into a priority-aware ready queue
   (unless `dont_filter` is set).
3. Request middlewares can enrich or rewrite requests before they are sent.
4. Worker tasks fetch requests through the HTTP client.
5. Response middlewares can transform the response or return a new request.
6. The response is parsed via a callback or the spider's `parse` method.
7. Outputs are turned into new requests or items, then re-queued or piped.
8. When the queue drains and no requests are pending, the engine shuts down.

Implementation entry points:
- Engine and core loop: `../src/engine.rs`
- Run helpers: `../src/runner.rs`

```rust
// Engine startup (simplified)
self.open_spider().await?;
self.await_idle().await;
self.shutdown().await;
```

## Engine Lifecycle

The engine handles startup and shutdown, including opening/closing pipelines
and spider hooks.

- `Engine::run` starts workers and optional stats logging.
- `open_spider` calls `Spider::open`, then opens pipelines and enqueues
  `start_requests` output.
- `close_spider` closes pipelines and calls `Spider::close`.

Relevant code:
- Engine lifecycle: `../src/engine.rs`
- Spider hooks: `../src/spider.rs`
- Item pipelines: `../src/pipelines.rs`

```rust
// open_spider (excerpt)
self.state.spider.open().await;
for pipe in &self.state.item_pipelines {
    pipe.open(self.state.spider.clone()).await?;
}
for req in self.state.spider.start_requests().await {
    self.enqueue(req).await?;
}
```

## Concurrency and Backpressure

- `HttpClient` uses a semaphore to cap concurrent HTTP requests.
- Incoming requests first go through a bounded `mpsc` channel sized by
  `max_pending_requests` (defaults to `concurrency * 10`) to provide
  backpressure.
- A dispatcher task moves requests into an internal priority queue consumed by
  workers (higher `Request.priority` first, FIFO for equal priorities).
- Scraped items are pushed to a separate bounded queue and processed by a
  dedicated item worker, so pipeline I/O does not block HTTP workers.

Relevant code:
- Queue creation and config: `../src/engine.rs`
- HTTP concurrency limit: `../src/http.rs`
- RunConfig defaults: `../src/runner.rs`

## Request De-Duplication

Silkworm maintains a `seen` set of request fingerprints. The fingerprint uses
HTTP method + canonical URL (including merged query params). If a request is
not marked as `dont_filter`, the engine skips duplicates. You can optionally
cap the set with `max_seen_requests` to bound memory.

```rust
// enqueue (excerpt)
if !req.dont_filter {
    let mut seen = self.state.seen.lock().await;
    if !seen.insert_if_new(&request_fingerprint(&req)) {
        return Ok(());
    }
}
```

Relevant code:
- De-duplication and enqueue: `../src/engine.rs`
- Request flag: `../src/request.rs`

## Request Handling

Request middlewares run before a request is sent. They can add headers,
timeouts, proxies, or meta values. The HTTP client merges default headers
with request headers and applies a request-specific timeout when set. If
`keep_alive` is enabled, it injects `Connection: keep-alive` when the header
is not already present.
Delay middleware now schedules delayed requeue via request metadata, so workers
are not blocked by `sleep`.

```rust
for mw in &self.state.request_middlewares {
    req = mw.process_request(req, self.state.spider.clone()).await;
}
self.state.stats.requests_sent.fetch_add(1, Ordering::SeqCst);
let resp = self.state.http.fetch(req).await?;
```

Relevant code:
- Request middleware chain: `../src/engine.rs`
- Request middleware traits: `../src/middlewares.rs`
- HTTP options and headers: `../src/http.rs`
- Request fields: `../src/request.rs`

## Response Handling

Responses pass through response middlewares. A middleware can:

- Return `ResponseAction::Response` to continue processing a response.
- Return `ResponseAction::Request` to re-queue a request (e.g., retries).
  Retries can carry delay metadata and be scheduled without blocking workers.

If no callback is defined, the engine wraps the response in `HtmlResponse` and
calls `Spider::parse`.

```rust
match processed {
    ResponseAction::Request(req) => self.enqueue(req).await?,
    ResponseAction::Response(resp) => {
        let outputs = if let Some(cb) = resp.request.callback.clone() {
            cb(self.state.spider.clone(), resp).await
        } else {
            self.state.spider.parse(resp.into_html(self.state.html_max_size_bytes)).await
        };
        for output in outputs? {
            // enqueue requests / run item pipelines
        }
    }
}
```

Relevant code:
- ResponseAction: `../src/middlewares.rs`
- Response handling: `../src/engine.rs`
- HTML wrapping: `../src/response.rs`

## Stats and Observability

The engine tracks counters for requests sent, responses received, items scraped,
errors, pending requests, and seen URLs. On Linux it also reports memory usage.

Relevant code:
- Stats collection and logging: `../src/engine.rs`
- Structured logger: `../src/logging.rs`
