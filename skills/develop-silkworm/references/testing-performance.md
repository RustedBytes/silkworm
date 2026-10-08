# Regression tests and performance work

Sources: docs/development.md, docs/benchmarks-criterion.md, benches/ and
module-local tests in src/engine.rs, src/middlewares.rs and src/pipelines.rs.

Start with saved HTML fixtures and assert exact item fields, missing fields,
Unicode handling, relative pagination URLs and allowed-origin rejection.
Exercise callbacks separately from parse. Use local in-process HTTP servers
for the engine: assert actual request counts, middleware headers, output items
and pipeline close/flush. Bound server reads and joins; avoid live sites and
sleep-based synchronization. Existing scripts/run_examples_offline.sh exercises
repository examples without public HTTP requests.

Add regressions for retryable versus non-retryable status, exhausted attempts,
transport errors, duplicate URLs, pending overload, bounded seen eviction,
oversized/truncated HTML, invalid config, pipeline failure and lifecycle order.
Test custom pipeline transformation order and failure propagation, not just Ok.
Use xpath and no-default-features configurations when affected; all-features
also requires the charset-detection toolchain (currently >=1.98).

Measure before optimizing. Separate HTTP latency, parsing, scheduling and file
output bottlenecks. Record commit, lockfile, features, compiler, CPU, dataset,
concurrency and warmups; compare medians and tail latency over repeated runs.
Use cargo bench --bench core --features xpath and focused Criterion benches;
consult docs/benchmarks-criterion.md for available groups. Use the repository
allocation harnesses for supported cases. Do not infer end-to-end gains from
selector microbenchmarks, or promise zero allocations for owned HTML/JSON.
Reuse HTTP clients and selectors, avoid unnecessary clones, bound output
vectors and move CPU/blocking work off async workers when measured worthwhile.
Preserve public behavior, feature parity, errors and ordering. Reject a speedup
that changes extraction results or hides failures. Repeat affected benchmarks
and deterministic tests after the change; report uncertainty and tradeoffs.
