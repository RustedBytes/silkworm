# Criterion public API benchmarks

`criterion_api` complements the existing custom harnesses without changing the
library API. Criterion 0.8.2 requires Rust 1.86; this crate requires Rust 1.92.
Fixtures are synthetic and deterministic (no RNG, remote HTTP, or filesystem I/O).

```sh
cargo bench -p silkworm-rs --bench criterion_api --no-run
cargo test -p silkworm-rs --bench criterion_api
cargo bench -p silkworm-rs --bench criterion_api
cargo bench -p silkworm-rs --bench criterion_api --features xpath
cargo bench -p silkworm-rs --bench criterion_api --no-default-features
```

| Group | Workloads | Timed boundary |
| --- | --- | --- |
| request | Fluent builder; clone with 0/8/64 headers and a shared 4 KiB Bytes body | Allocation, construction/clone and output destruction |
| decode | UTF-8 and Windows-1252, approximately 1 KiB/64 KiB/1 MiB | Public text decoding, returned String allocation and destruction |
| html | 10/200/2000 product rows; fresh response; warm compiled/string CSS; invalid CSS; optional compiled/string XPath | Fresh includes response construction, decoding, DOM parse, selection and destruction; warm excludes initial CSS DOM parsing |
| links | Resolve 1/100/1000 relative links with query and fragment | URL resolution, Request and Vec creation and destruction |

The product fixture is well-formed XML-compatible HTML without a doctype, because
the public XPath helper parses its source as XML. It is not representative of
arbitrary malformed web HTML. Assertions run outside timing. HTML sizes stay below the 2 MB document limit.
`Throughput::Bytes` uses actual input bytes; link throughput counts resolved URLs.
Each request benchmark iteration is one request. All loops use `iter`, including
output destruction. No generated input or correctness assertions run inside timing.
`std::hint::black_box` protects runtime inputs and returned values.

Warm HTML uses the same response and precompiled selector on every iteration.
With default `scraper-atomic`, its CSS DOM cache is warmed before measurement.
Without that feature, CSS reparses the DOM: compare only matching feature sets.
XPath may still rebuild its own document; compiled XPath excludes only query
compilation, not document evaluation. Byte throughput for warm CSS is a normalized
input-size rate, not a claim that all input bytes are decoded each iteration.
These CPU microbenchmarks do not measure crawler networking, concurrency, filesystem
pipelines, request tail latency, or allocations. Wall time cannot prove zero-copy
or zero-allocation. Existing custom harnesses remain unchanged.

## Reproducible comparisons

Use normal Criterion defaults (100 samples, 3 s warm-up, 5 s measurement, 95%
confidence intervals). Smoke/test/quick modes are validation only. Reports appear
under `target/criterion`; never commit them. On the same idle host, apply the same
harness and fixtures to both revisions and use a common `CARGO_TARGET_DIR`:

```sh
cargo bench -p silkworm-rs --bench criterion_api -- --save-baseline base-UNIQUE_SHA
# Switch to candidate, retaining the identical harness and baseline directory.
cargo bench -p silkworm-rs --bench criterion_api -- --baseline base-UNIQUE_SHA
```

Never overwrite a reference baseline. Record both SHAs, dirty diff, Cargo.lock,
`rustc -Vv`, CPU/OS, allocator, features, profile, RUSTFLAGS and commands. Repeat
material comparisons in alternating order. Treat a 5% time change as a preliminary
practical threshold; require repeatable confidence intervals before claiming a
regression or improvement. This is not a CI pass/fail gate. Criterion confidence
intervals describe operation time estimates, not request p95/p99.
