# Development and documentation verification

## Build the current checkout

The repository uses Rust edition 2024 with MSRV 1.92. These instructions target
Linux; other platforms have not been verified by this guide. Build dependencies
for the native TLS stack are a C/C++ compiler, CMake and libclang with headers
available to bindgen. On Ubuntu, install them with:

```bash
sudo apt-get update
sudo apt-get install build-essential cmake clang libclang-dev pkg-config
rustup toolchain install 1.92.0 --component rustfmt --component clippy
```

Then fetch and build the source rather than a published crate:

```bash
git clone https://github.com/RustedBytes/silkworm.git
cd silkworm
cargo +1.92.0 check --locked --all-targets
```

If bindgen cannot locate libclang, point `LIBCLANG_PATH` to the directory
containing `libclang.so`. Keep the compiler's system headers available too.
The native build can consume substantial disk space; subsequent builds reuse
Cargo's target directory.

## Verify documentation

Python 3 is needed for the Markdown example checker:

```bash
RUSTDOCFLAGS="-D warnings" cargo +1.92.0 doc --locked --no-deps --all-features
cargo +1.92.0 test --locked --doc --all-features
RUSTUP_TOOLCHAIN=1.92.0 python3 scripts/check_docs.py
```

The checker checks local links, heading anchors and code fences, then extracts every `rust` block from the README and guides, supplies
the fragment context described in the [index](README.md#example-conventions),
and compiles against the current public exports with default, all and no
optional features. Complete programs compile in their own modules; statement
fragments receive an async function, a small spider and an HTML response.
Architecture blocks labelled `text` are illustrative pseudocode, not examples.

The original README quickstart is compiled without changing its start URL.
An additional copy is executed against a local HTTP fixture: the
checker substitutes a local `start_requests` implementation for its external
`start_urls`. The parsing and runner code is unchanged. It does not verify
availability or content of the external demo website. Temporary integration
sources are removed after the check, including on failure.

Run the existing eleven repository examples using local fixtures:

```bash
RUSTUP_TOOLCHAIN=1.92.0 ./scripts/run_examples_offline.sh
```

The script sets offline mode itself and supplies `xpath`/`cli-examples` flags
where required. Its export files go under `/tmp`; no live websites are used.

## Code checks

For code changes, the [CI workflow](../.github/workflows/test.yml) runs:

```bash
cargo +1.92.0 fmt --all -- --check
cargo +1.92.0 check --all-targets
cargo +1.92.0 test --all
cargo +1.92.0 test --all-features
cargo +1.92.0 test --no-default-features
cargo +1.92.0 clippy --all-targets --all-features -- -D warnings
```

The core benchmark threshold job is scheduled and optionally manually
requested; it is separate from normal pull-request tests. Microbenchmarks and
allocation boundaries are described in [architecture](architecture.md) and
[HTTP documentation](http-and-logging.md). Reduced allocation counts alone do
not establish faster whole-crawl throughput.
