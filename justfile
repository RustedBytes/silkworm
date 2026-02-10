set shell := ["bash", "-euo", "pipefail", "-c"]

default:
  @just --list

# Baseline tests (matches CI).
test:
  cargo test --all

# Format all Rust code.
fmt:
  cargo fmt --all

# Verify code is already formatted.
fmt-check:
  cargo fmt --all -- --check

# Compile every target.
check:
  cargo check --all-targets

# Run tests with all optional features.
test-all-features:
  cargo test --all-features

# Run tests without default features.
test-no-default-features:
  cargo test --no-default-features

# Run the benchmark harness (requires xpath).
bench-core:
  cargo bench --bench core --features xpath

# Common examples.
example-quotes:
  cargo run --example quotes_spider

example-quotes-xpath:
  cargo run --example quotes_spider_xpath --features xpath

example-export pages="2":
  cargo run --example export_formats_demo --features cli-examples -- --pages {{pages}}

# Full local verification matrix.
ci: fmt-check check test test-all-features test-no-default-features
