#!/usr/bin/env bash
set -euo pipefail

cargo test --locked --all
cargo test --locked --features xpath,cli-examples
cargo test --locked --no-default-features
python3 scripts/check_docs.py
python3 skills/develop-silkworm/scripts/check_examples.py --repo .
bash scripts/run_examples_offline.sh
