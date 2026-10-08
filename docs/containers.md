# Docker parser and test environment

Build the default image from the repository root:

```bash
docker build -t silkworm-parser .
docker run --name silkworm-demo --network none silkworm-parser
docker cp silkworm-demo:/app/data/quotes.jl ./quotes.jl
python3 scripts/check_quotes_output.py ./quotes.jl
docker rm silkworm-demo
```

The default image runs the existing `quotes_spider` example as UID/GID 10001.
It starts its in-process mock server, follows pagination across two HTML pages,
writes three quotes to `/app/data/quotes.jl`, flushes the pipeline and exits.
No host port is needed. `--network none` permits container loopback while
preventing external HTTP. Building the image requires network access for the
base images, Debian packages and locked Cargo dependencies.

For persistent output, use a named volume:

```bash
docker volume create silkworm-quotes
docker run --rm --network none -v silkworm-quotes:/app/data silkworm-parser
```

JSONL appends on each run. The exact three-item check requires a fresh output
file/container, not an existing volume with earlier results. Bind mounts must
be writable by UID/GID 10001; pre-create an appropriate directory or choose
an explicit `--user` matching its owner. Do not recursively change ownership
of unrelated host directories.

To parse the public demo site instead, explicitly enable external access:

```bash
docker run --rm -e SILKWORM_EXAMPLE_OFFLINE=0 silkworm-parser
```

This uses `https://quotes.toscrape.com/` from the example; it is not a generic
URL CLI. Modify the spider or build your own binary for another source.
Control logging with `-e SILKWORM_LOG_LEVEL=DEBUG`. The output file stays in the
container unless mounted or copied before removal.

## Run the repository tests

Build and run the optional test target (keeps the compiler and source):

```bash
docker build --target test -t silkworm-tests .
docker run --rm silkworm-tests
```

The test command runs baseline, XPath/CLI and no-default feature tests,
documentation checks, bundled skill examples and the offline example suite.
Its HTTP fixtures are local; Cargo may still need network access to download
optional dependencies. Override CMD for a focused command, for example:

```bash
docker run --rm silkworm-tests cargo test --locked --lib
```

The builder uses Rust 1.92 (the default-feature MSRV), CMake, C/C++ tools,
libclang, Perl and Python. The runtime uses compatible Debian Bookworm,
CA certificates and libstdc++ without compiler tooling. To enable
`charset-detection`, use a Rust >=1.98 build argument and add that feature to
your test/build command; this Dockerfile does not enable it implicitly.
Base tags and Debian package revisions can change; Cargo.lock fixes Rust
dependency resolution. Pin approved base-image digests for stricter deployment
reproducibility. No registry publishing is configured.

The container CI builds the final image, runs with external networking disabled
and asserts the complete JSONL output. Local validation without an installed
Docker daemon only validates the parser/configuration, not image execution.
