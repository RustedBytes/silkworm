# Middlewares

Silkworm supports request and response middleware stacks. Each middleware is
async and can mutate requests, short-circuit responses, or inject retries.

## Middleware Traits

- `RequestMiddleware`: transforms an outgoing request before it is fetched.
- `ResponseMiddleware`: transforms a response or returns a new request.

Code:
- Middleware traits and actions: `../src/middlewares.rs`

```rust
use silkworm::middlewares::MiddlewareFuture;

struct TraceMiddleware;

impl<S: Spider> RequestMiddleware<S> for TraceMiddleware {
    fn process_request<'a>(
        &'a self,
        mut request: Request<S>,
        _spider: Arc<S>,
    ) -> MiddlewareFuture<'a, Request<S>> {
        Box::pin(async move {
            request.headers.insert("X-Trace".to_string(), "1".to_string());
            request
        })
    }
}
```

## Built-In Request Middlewares

### UserAgentMiddleware

Assigns a User-Agent header when one is missing. You can pass a list of agents
for random rotation or a single default value.

Code:
- `UserAgentMiddleware`: `../src/middlewares.rs`

### ProxyMiddleware

Adds a proxy URL to `request.meta["proxy"]` (`request::meta_keys::PROXY`).
Code paths now use typed accessors (`Request::with_proxy` / `Request::proxy`)
instead of manual `serde_json::Value` extraction.
The HTTP client uses this value
to build (and cache) a proxy-configured wreq client.

Code:
- `ProxyMiddleware`: `../src/middlewares.rs`
- Proxy-aware client: `../src/http.rs`

### DelayMiddleware

Adds a delay before a request is sent. Strategies include:

- Fixed delay
- Random range delay
- Custom per-request function

Code:
- `DelayMiddleware`: `../src/middlewares.rs`

## Built-In Response Middlewares

### RetryMiddleware

Retries responses based on status codes and exponential backoff.

Behavior details:
- Uses `request.meta["retry_times"]` (`request::meta_keys::RETRY_TIMES`) to
  track attempts via `Request::retry_times` / `Request::set_retry_times`.
- Marks retry requests as `dont_filter` to bypass de-duplication.
- Stores backoff delay metadata so the engine can schedule delayed retries
  without blocking worker execution (`Request::set_retry_delay_secs`).

Code:
- `RetryMiddleware`: `../src/middlewares.rs`

### SkipNonHtmlMiddleware

Skips non-HTML responses unless `request.meta["allow_non_html"]`
(`request::meta_keys::ALLOW_NON_HTML`) is true
(`Request::allow_non_html` helper).
It can also drop the body to reduce memory usage and replaces the callback
with a no-op to avoid extra parsing.

Code:
- `SkipNonHtmlMiddleware`: `../src/middlewares.rs`
- HTML sniffing: `../src/response.rs`

```rust
use silkworm::{
    DelayMiddleware, RetryMiddleware, RunConfig, SkipNonHtmlMiddleware, UserAgentMiddleware,
};

let config = RunConfig::new()
    .with_request_middleware(UserAgentMiddleware::new(
        vec![],
        Some("silkworm-rs/docs-example".to_string()),
    ))
    .with_request_middleware(DelayMiddleware::fixed(0.25))
    .with_response_middleware(RetryMiddleware::new(3, None, None, 0.5))
    .with_response_middleware(SkipNonHtmlMiddleware::new(None, 1024));
```
