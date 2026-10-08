# Keep builder and runtime on the same Debian release (glibc ABI).
ARG RUST_VERSION=1.92
FROM rust:${RUST_VERSION}-bookworm AS build

# wreq/btls-sys builds BoringSSL with CMake and generates bindings with libclang.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential ca-certificates clang cmake libclang-dev perl pkg-config python3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
ENV CARGO_INCREMENTAL=0
COPY . .
RUN cargo build --locked --release --example quotes_spider

# Optional development/test image: docker build --target test ...
FROM build AS test
ENV SILKWORM_EXAMPLE_OFFLINE=1
CMD ["bash", "scripts/test_container.sh"]

# Default image runs the compiled demo, without Cargo or build tools.
FROM debian:bookworm-slim AS parser
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libstdc++6 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 crawler \
    && useradd --uid 10001 --gid crawler --no-create-home crawler \
    && mkdir -p /app/data \
    && chown crawler:crawler /app/data
COPY --from=build /src/target/release/examples/quotes_spider /usr/local/bin/quotes_spider
WORKDIR /app
USER 10001:10001
ENV SILKWORM_EXAMPLE_OFFLINE=1 SILKWORM_LOG_LEVEL=INFO
ENTRYPOINT ["/usr/local/bin/quotes_spider"]
