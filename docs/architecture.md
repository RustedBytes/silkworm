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
8. When the request queues drain and no requests or items are pending, the
   engine shuts down
   (or earlier when `fail_fast` is enabled and an error occurs).

Implementation entry points:
- Engine and core loop: `../src/engine.rs`
- Run helpers: `../src/runner.rs`

```text
// Engine startup (simplified)
self.open_spider().await?;
self.await_idle_or_worker_health(...).await?;
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

```text
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
- Requests enter the priority queue directly (higher `Request.priority` first,
  FIFO for equal priorities). No intermediate request channel or dispatcher
  can hide additional backlog.
- A configured `max_pending_requests` bounds ready and delayed requests
  together, excluding active workers. A slot is released when a worker takes
  a request. Overload stops the crawl with an explicit capacity error because
  blocking workers that also produce requests could deadlock the crawl.
- Ownership guards release pending counts and backlog slots on completion,
  cancellation and queue cleanup. Delayed requests keep their slots until
  consumed or cancelled.
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

The duplicate lookup happens before reserving a backlog slot. New fingerprints
are inserted only after admission succeeds; rejected requests do not enter the
history. FIFO eviction can make an old request eligible again. Body, headers,
metadata and callbacks are not part of the fingerprint.

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

```text
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

```text
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

### Item idle notifications

Item accounting wakes the coordinator only when the number of pending items
transitions to zero. Intermediate completions cannot satisfy the idle predicate.
The atomic decrement selects the final completion even across runtime threads;
the coordinator registers its notification before checking the count. Dropped
or cancelled items use the same ownership guard and release their count.
Request completion notifications remain unchanged because they also drive
cleanup of finished delayed-request tasks.

To measure this accounting path independently of HTTP and pipeline I/O, run
`cargo bench --bench item_accounting`. It uses the production ownership guard,
a burst of 200,000 admitted items, one or four completion producers, and both a
current-thread runtime and a four-worker runtime. Producers yield every 64
completions; each case has two warmups and nine reported samples. The measured
time includes task startup, accounting, notification waiting and joins, but
excludes constructing the admitted guards. This is a scheduler microbenchmark,
not a crawl throughput or bounded item-channel benchmark; it does not establish
performance for HTML parsing, slow pipelines, HTTP, or external overload.

### Deduplication storage

With `max_seen_requests` configured, membership and FIFO eviction share each
fingerprint through `Arc<str>`. A duplicate does not refresh its FIFO position.
Unbounded storage keeps singly owned `Box<str>` keys. Borrowed string lookup and
Rust's default randomized hashing are retained; fingerprints and equality are
unchanged. Sharing removes the second payload allocation and copy, but adds
reference-count operations and an allocation header, so cold insertion latency
and very short-key memory usage can differ.

`cargo bench --bench seen_requests` measures the production cache with 20,000
operations and 32-, 128- and 1024-byte fingerprints. Cases cover cold bounded
and unbounded filling, steady FIFO eviction at capacity 4096, and duplicate
hits. Fixtures and steady-cache initialization are excluded; cold filling
includes construction, growth and destruction. Each case has two warmups and
nine timed samples. The synchronous microbenchmark excludes URL fingerprint
construction, engine locks, HTTP and whole-crawl throughput.

For separate allocation measurements, run
`python scripts/measure_seen_allocations.py --toolchain 1.92.0`. It builds an
isolated temporary harness using `stats_alloc` 0.1.10 and the same workloads;
the project's dependency manifest and lockfile stay unchanged. Counts include
Rust System allocator requests, reallocations, deallocations and requested
bytes inside the workload boundary, not allocator usable size or peak RSS.
Fixture allocation, output formatting and steady-cache setup/teardown are
excluded; cold-cache destruction is included. Allocation instrumentation is
not used for latency samples. Duplicate-hit counts describe only cache lookup,
not fingerprint generation or the rest of a crawl.
