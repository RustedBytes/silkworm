# AGENTS Guide

This file is for coding agents working in `silkworm-rs`.
It describes the codebase shape, invariants, and the minimum verification
expected before proposing changes.

## Project Snapshot

- Crate package: `silkworm-rs`
- Library crate name: `silkworm`
- Rust edition: `2024`
- Minimum Rust version: `1.92`
- Main purpose: async-first scraping framework (engine + spider API +
  middleware/pipeline system)

## Fast Commands

- Baseline (matches CI): `cargo test --all`
- Format check/fix: `cargo fmt --all`
- Full feature check: `cargo test --all-features`
- No-default feature check: `cargo test --no-default-features`
- All targets compile check: `cargo check --all-targets`
- Bench harness (XPath needed): `cargo bench --bench core --features xpath`

Examples:
- Basic example: `cargo run --example quotes_spider`
- XPath example: `cargo run --example quotes_spider_xpath --features xpath`
- CLI example: `cargo run --example export_formats_demo --features cli-examples -- --pages 2`

## Repository Map

- Public API exports: `src/lib.rs`
- Prelude exports: `src/prelude.rs`
- Spider trait: `src/spider.rs`
- Runtime config and entrypoints: `src/runner.rs`
- Engine core loop and lifecycle: `src/engine.rs`
- HTTP layer: `src/http.rs`
- Request model and callbacks: `src/request.rs`
- Response/HTML/XPath helpers: `src/response.rs`
- Middleware traits and built-ins: `src/middlewares.rs`
- Item pipeline traits and built-ins: `src/pipelines.rs`
- Shared types and conversion helpers: `src/types.rs`
- Error model: `src/errors.rs`
- Logger: `src/logging.rs`
- Utility fetch API: `src/api.rs`
- User-facing docs: `docs/*.md`
- Runnable examples: `examples/*.rs`
- Benchmarks: `benches/core.rs`

## Architectural Invariants

1. Engine lifecycle is: open spider -> open pipelines -> enqueue start requests
   -> run workers -> drain pending queue -> close pipelines -> close spider.
2. Request de-duplication is based on `Request.url` unless `dont_filter` is
   true.
3. Response middlewares may return:
   - `ResponseAction::Response` to continue normal parse flow.
   - `ResponseAction::Request` to enqueue a new request immediately.
4. If a request has `callback`, engine uses that callback instead of
   `Spider::parse`.
5. Item pipelines run sequentially in configured order for every emitted item.
6. `html_max_size_bytes` limits how much response body is decoded/parsed into
   `HtmlResponse`.
7. `HttpClient::new` requires `concurrency > 0`.

## Coding Rules for Agents

- Keep `#![forbid(unsafe_code)]` expectations intact. Do not introduce `unsafe`.
- Prefer adding or updating unit tests in the same module (`#[cfg(test)]`)
  where behavior is changed.
- Preserve feature-gated behavior:
  - `xpath` gates XPath support.
  - `scraper-atomic` changes HTML caching behavior.
  - `cli-examples` gates clap-based examples.
- Keep error surfaces coherent with `SilkwormError` categories.
- Keep logging structured (`key=value` fields) via `Logger`.
- Avoid network-dependent tests; existing tests use local in-process servers.

## Consistency Checklist by Change Type

- If changing public API:
  - Update exports in `src/lib.rs`.
  - Update `src/prelude.rs` when type should be part of ergonomic imports.
  - Update docs in `README.md` and relevant files under `docs/`.
- If changing `RunConfig` or engine config fields:
  - Update `RunConfig`, `EngineConfig`, and `From<RunConfig<S>> for EngineConfig<S>`.
  - Update associated tests in `src/runner.rs` and `src/engine.rs`.
- If changing request/response models:
  - Keep clone/debug behavior in sync.
  - Update helper methods and related tests in `src/request.rs` and `src/response.rs`.
- If changing middleware or pipeline behavior:
  - Update docs sections: `docs/middlewares.md` or `docs/pipelines.md`.
  - Add behavior-focused tests near the modified middleware/pipeline.

## Known Gotchas

- `Request.priority` is stored but currently not used by engine scheduling.
- De-dup happens before URL parameter merge in `HttpClient`; two requests with
  the same `url` but different `params` will still be considered duplicates
  unless `dont_filter` is set.
- `SkipNonHtmlMiddleware` can clear body/headers and replace callback with
  no-op; parsing assumptions must account for this.
- XPath helpers should gracefully report disabled-feature errors when `xpath`
  feature is off.
