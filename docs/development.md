# Development and documentation verification

## Build the current checkout

The repository uses Rust edition 2024 with MSRV 1.92 for default builds. The optional
`charset-detection` feature requires Rust 1.98. These instructions target
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
RUSTDOCFLAGS="-D warnings" cargo +1.92.0 doc --locked --no-deps --features xpath,cli-examples
cargo +1.92.0 test --locked --doc --features xpath,cli-examples
RUSTUP_TOOLCHAIN=1.92.0 python3 scripts/check_docs.py
```

The checker checks local links, heading anchors and code fences, then extracts every `rust` block from the README and guides, supplies
the fragment context described in the [index](README.md#example-conventions),
and compiles against the current public exports with default, MSRV-compatible optional features and no default
features. On Rust 1.98 or newer, pass `--all-features` to the checker to include
charset detection. Complete programs compile in their own modules; statement
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
cargo +1.92.0 test --features xpath,cli-examples
cargo +1.92.0 test --no-default-features
cargo +1.92.0 clippy --all-targets --features xpath,cli-examples -- -D warnings
```

CI separately runs `cargo test --locked --all-features` and
`cargo test --locked --no-default-features --features charset-detection` on Rust 1.98,
and lints all features with that toolchain.

The core benchmark threshold job is scheduled and optionally manually
requested; it is separate from normal pull-request tests. Microbenchmarks and
allocation boundaries are described in [architecture](architecture.md) and
[HTTP documentation](http-and-logging.md). Reduced allocation counts alone do
not establish faster whole-crawl throughput.

## Update the package version

After the workflow is merged, open Actions → Update version → Run workflow on
`master`. Choose `patch`, `minor` or `major`, or enter an explicit stable
`MAJOR.MINOR.PATCH` version. An explicit version takes precedence and must be
strictly greater than the current version; leading zeros, prerelease suffixes and
`v` prefixes are rejected.

The workflow updates only the root package version in `Cargo.toml` and `Cargo.lock`,
validates Cargo metadata, and opens a `chore/version-X.Y.Z` pull request. Existing
version branches or `vX.Y.Z` tags cause a failure rather than being overwritten.
GitHub Actions must be allowed to create pull requests under Settings → Actions →
General → Workflow permissions. The workflow explicitly dispatches `test.yml` on
its new branch, so tests can run even when the branch is pushed with `GITHUB_TOKEN`.
Review and merge the version PR normally. This workflow does not create a tag,
publish to crates.io or create a GitHub Release.
