# Silkworm (Rust) Documentation

This folder describes the Silkworm scraping framework, its core concepts, and how
requests flow through the engine. Each section links directly to the
implementation in `src/` so the docs stay grounded in the code.

## Document Map

- [Architecture and data flow](architecture.md)
- [Core concepts (Spider, Request, Response)](core-concepts.md)
- [Middlewares](middlewares.md)
- [Pipelines](pipelines.md)
- [Configuration and runtime](configuration.md)
- [HTTP client, utility API, and logging](http-and-logging.md)
- [Errors and shared types](errors-and-types.md)
- [Implementation roadmap](implementation-roadmap.md)

## Module Map

- Public re-exports: `../src/lib.rs`
- Spider trait: `../src/spider.rs`
- Engine internals: `../src/engine.rs`
- Run helpers: `../src/runner.rs`
- Requests and callbacks: `../src/request.rs`
- Responses and selectors: `../src/response.rs`
- Middlewares: `../src/middlewares.rs`
- Pipelines: `../src/pipelines.rs`
- HTTP client: `../src/http.rs`
- Utility API: `../src/api.rs`
- Logging: `../src/logging.rs`
- Errors: `../src/errors.rs`
- Shared types: `../src/types.rs`
- Prelude re-exports: `../src/prelude.rs`
