# Silkworm (Rust) Documentation

This folder describes the Silkworm scraping framework, its core concepts, and how
requests flow through the engine. Each section links directly to the
implementation in `src/` so the docs stay grounded in the code.

## Document Map

- [Installation and optional features](installation.md)
- [Architecture and data flow](architecture.md)
- [TL integration and parser evaluation (#17)](parser-evaluation.md)
- [Core concepts (Spider, Request, Response)](core-concepts.md)
- [Middlewares](middlewares.md)
- [Pipelines](pipelines.md)
- [Configuration and runtime](configuration.md)
- [HTTP client, utility API, and logging](http-and-logging.md)
- [Errors and shared types](errors-and-types.md)
- [Development, runnable examples and benchmarks](development.md)
- [Criterion benchmark workloads](benchmarks-criterion.md)

## Example conventions

The [project README](../README.md#quick-start) contains the complete
`QuotesSpider` application. Guide snippets
are fragments: use that definition, add `use silkworm::*;` (the common snippet context)
and any explicit imports shown, and put statements
inside a function returning `SilkwormResult<()>`. Async fragments require a
Tokio runtime; selector snippets assume an `HtmlResponse<QuotesSpider>` named
`response`. Finish statement fragments with `Ok(())`. The output-model fragment
instead belongs in `Spider::parse` and returns `SpiderResult<Self>`.

API signatures and feature availability come from the current checkout, not
from a planned release. See the development guide for checks against this code.

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
