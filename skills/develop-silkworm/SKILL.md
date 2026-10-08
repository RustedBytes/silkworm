---
name: develop-silkworm
description: Build, debug, test and optimize Rust crawlers using silkworm-rs (crate silkworm). Use for Spider implementations, CSS/XPath extraction, request callbacks, middleware, item pipelines, concurrency, retries, resource limits, logging and crawling regressions.
---

# Develop Silkworm applications

## Resolve the API before editing

Read the target project's AGENTS.md and Cargo.toml/Cargo.lock. Run
`cargo metadata --locked --format-version 1` and identify the resolved
silkworm-rs version, source and enabled features; do not assume the latest
published API equals the installed API. For a framework checkout, inspect
src/lib.rs, src/prelude.rs and the implementation files named below.
For a consumer, inspect the resolved Cargo registry/git source or versioned
https://docs.rs/silkworm-rs/<resolved-version>/silkworm/ documentation.
Treat this bundle as a 0.2.1 baseline. Recheck signatures and defaults after
any version/source change, compile examples against that source, and update
references and tests together. Query upstream releases only when an upgrade
is requested; report the exact version/commit used. Do not upgrade implicitly.

## Choose and implement the crawl

1. Define the item schema, seed URLs, allowed origins, pagination termination,
   timeout, output destination and expected failure behavior.
2. Use `crawl_with` inside Tokio; use `run_spider_with` in synchronous code.
3. Implement `Spider::start_requests` for owned/dynamic seeds and `parse` for
   extraction. Return both items and follow requests as SpiderOutput values.
   Validate links against allowed origins before enqueuing untrusted links.
   Use fallible selectors during development instead of hiding invalid CSS.
4. Read [API and extraction](references/api.md) for callbacks and selectors.
5. Read [middleware and pipelines](references/extensions.md) before adding
   hooks. Register transformations in intentional order and propagate errors.
6. Read [runtime and limits](references/runtime.md) before selecting concurrency,
   retries or storage. Start bounded, measure, and increase one setting at a time.
7. Read [testing and performance](references/testing-performance.md) before
   changing hot paths or adding regression coverage.

## Verify and deliver

Use [the practical example](assets/crawler.rs) as a complete integration-test
and crawler template. It includes custom request/response middleware, a custom
pipeline, bounded config, CSS pagination and feature-gated XPath assertions.
Run `python3 skills/develop-silkworm/scripts/check_examples.py --repo .`
in the framework checkout to test this exact asset under three feature sets.
Read [example usage](references/examples.md) to adapt it to another project.
Run formatter, all-target compilation, baseline tests and relevant feature tests
as required by AGENTS.md; run scripts/check_docs.py for documentation changes.
Do not describe compilation as runtime coverage or local fixtures as live-site
validation. Report version, features, commands, outcomes and remaining limits.

## Install this repository skill

Copy the entire `skills/develop-silkworm` folder into an agent's skill directory
(for Codex, `$CODEX_HOME/skills` or `~/.codex/skills`). Keep assets, references
and scripts alongside SKILL.md. Use `$develop-silkworm` in a task prompt.
For ChatGPT, import the complete folder through the skill management UI when
available; do not paste SKILL.md alone and discard its resources.
