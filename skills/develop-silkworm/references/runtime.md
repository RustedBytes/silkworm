# Runtime, limits and logging

Sources: src/runner.rs, src/engine.rs, src/http.rs, src/logging.rs,
docs/configuration.md and docs/http-and-logging.md.

| RunConfig field | 0.2.1 default | Operational consequence |
| --- | --- | --- |
| concurrency | 16 | Must be positive; worker/HTTP concurrency |
| request_timeout | None | Set an explicit bound; request override wins |
| max_pending_requests | None | Ready + delayed backlog unbounded |
| max_seen_requests | Some(100000) | FIFO eviction permits old URLs again |
| html_max_size_bytes | 5000000 | Retained bytes, not total download bytes |
| fail_fast | false | Processing errors can coexist with Ok completion |
| log_stats_interval | None | Explicitly enable periodic statistics |
| keep_alive | false | Header injection setting, not a throughput guarantee |

Use with_concurrency, with_request_timeout, with_max_pending_requests,
with_max_seen_requests, with_html_max_size_bytes and with_log_stats_interval.
Pending and seen caps must be positive. Pending overload aborts even when
fail_fast is false; it is not blocking backpressure or silent drop. Active
requests, output vectors, item channels, parser state and dedup history have
separate memory costs. Limit fan-out as well as backlog. Body truncation can
produce incomplete HTML, and the client drains the remainder; timeouts still
matter. Unbounded seen history trades stronger deduplication for growing RAM.

Configure SILKWORM_LOG_LEVEL before starting the process (INFO default).
Use spider.log().info/debug/warn/error with key=value fields; record status,
counts, elapsed time and errors without credentials or sensitive payloads.
Await crawl_with to let pipelines flush. Do not call sync runtime helpers
inside Tokio. A successful non-fail-fast result does not prove all items saved.
