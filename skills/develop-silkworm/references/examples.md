# Run and adapt the practical example

The complete assets/crawler.rs is compiled as an integration test by
scripts/check_examples.py. Its `crawl_catalog(seed, output)` entrypoint uses
async crawl_with; call it from your Tokio application. Change fixture selectors
and item schema for your target. Restrict pagination origins as demonstrated.
The seed is caller supplied; the asset itself never starts public HTTP requests.

For a consumer project, copy the asset into tests/, add bytes, serde_json, url
and tokio test dependencies, and depend on silkworm-rs with lib name silkworm.
Declare/forward an xpath feature if keeping the feature-gated XPath test.
Run cargo test --test <copied-file-stem>. For a complete runnable CLI and
local server-backed examples, consult examples/quotes_spider.rs and
examples/callback_pipeline_demo.rs in the resolved framework checkout.

The bundle checker requires Python >=3.11 (tomllib) and Cargo, plus a framework
checkout. It prints its version, compiles and executes the exact asset with
default, xpath and no-default features, then removes its temporary target.
Cargo may download/build dependencies; offline describes HTTP fixtures,
not guaranteed offline dependency installation. It never updates the lockfile.
