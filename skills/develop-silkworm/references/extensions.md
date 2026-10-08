# Middleware and pipeline recipes

Sources: src/middlewares.rs, src/pipelines.rs, docs/middlewares.md,
docs/pipelines.md and examples/callback_pipeline_demo.rs.

Implement RequestMiddleware<S>::process_request with Arc<S> and a boxed
MiddlewareFuture yielding Request<S>. Implement ResponseMiddleware similarly,
yielding ResponseAction::Response to parse or ResponseAction::Request to enqueue.
Inspect the exact lifetimes in the trait before copying the asset.
Request middleware runs on retries too; make mutations idempotent.
Register RetryMiddleware before a response transform that discards status/body.
RetryMiddleware::new(3, None, None, 0.5) permits three additional HTTP attempts;
its default statuses include 429 and common server errors. It cannot retry
transport failures. Backoff is base * 2^n for delayed statuses, and retries
bypass deduplication. Do not claim jitter, Retry-After handling or a per-host
rate limiter without checking/implementing those policies.
DelayMiddleware uses delayed scheduling; a fixed delay is not a global
requests-per-second guarantee with multiple workers.
SkipNonHtmlMiddleware may discard the body and install a no-op callback;
set allow_non_html on requests that intentionally handle non-HTML data.

Implement ItemPipeline<S> with boxed PipelineFuture results. open/close manage
resources; process_item returns the transformed Item for the next pipeline.
Use CallbackPipeline for simple functions. Avoid blocking I/O and holding locks
across await. Make external writes idempotent if reruns/retries can repeat items.
File writers have bounded command queues (16), serialize admitted records,
and flush on close. Await normal crawl completion. Dropping a crawl future
is not guaranteed graceful shutdown. Buffered acknowledgement is not fsync.
JSONL appends; CSV/XML truncate on open. CSV fields inferred from the first
item do not expand automatically. Choose an explicit schema for stable exports.

After retries are exhausted, the response continues to parsing. Reject unwanted
final statuses explicitly in parse/callbacks; fail_fast alone does not reject HTTP
status codes. The asset demonstrates this policy.
