#!/usr/bin/env bash
set -euo pipefail

export SILKWORM_EXAMPLE_OFFLINE=1
export SILKWORM_LOG_LEVEL=error

echo "Running examples in offline mock-server mode"

cargo run --example callback_pipeline_demo
cargo run --features cli-examples --example export_formats_demo -- --pages 1 --output-dir /tmp/silkworm-export
cargo run --features cli-examples --example url_titles_spider -- --output /tmp/silkworm-url-titles.jl
cargo run --features cli-examples --example hackernews_spider -- --pages 1
cargo run --features cli-examples --example lobsters_spider -- --pages 1
cargo run --features xpath --example quotes_spider_xpath
cargo run --example quotes_spider_ergonomic
cargo run --example hybrid_logger_demo
cargo run --features cli-examples --example sitemap_spider -- --pages 2 --output /tmp/silkworm-sitemap.jl --concurrency 1
cargo run --example logger_configuration_demo
cargo run --example quotes_spider
