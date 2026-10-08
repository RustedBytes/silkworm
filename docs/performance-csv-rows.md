# CSV row encoding and redirect history review

Measured on 2026-10-08, against the CSV row encoder at
`5fb1d5bb0085f841713ae7e2a43fd783b6e6fceb`. Public APIs, dependencies,
MSRV, field ordering, header behavior and CSV bytes are unchanged.

## Confirmed correctness defect

`HttpClient::fetch` previously allocated a `Vec<String>` with capacity
`max_redirects.saturating_add(1)` before validating the URL. Setting
`max_redirects = usize::MAX` caused a `capacity overflow` panic, including
when redirects were disabled. Smaller, excessive limits could reserve
unnecessary memory for every request.

The history now starts empty and retains URLs only when a response is
actually followed as a redirect. Nonredirect responses need no history
allocation or URL clone. Limit enforcement and loop detection are preserved.
The regression test failed with the original allocation and passes with the
fix. Local HTTP tests cover a successful response with the large limit, both
redirect configurations, and self-loop detection.

## CSV change and measurement boundary

CSV escaping now appends borrowed fields directly to the writer's reusable
`Vec<u8>`, copying contiguous spans and doubling quotes. It replaces the
temporary strings created by `to_string`, `replace` and `format!`.

The benchmark includes the production private module and the previous row
encoder, and checks their output equality before measuring. Fixtures and an
output buffer large enough for the row are prepared outside the measured
region. Each operation clears that buffer, encodes one complete row, and
black-boxes the output. Buffer destruction is outside the measured region;
temporary strings in the previous implementation are destroyed within it.

Timing: Rust 1.92.0, x86_64 Linux, AMD EPYC 9V74, `opt-level=3`, fat LTO,
one codegen unit. Nine rounds alternate before/after for each workload;
20,000 rows per sample, two warmup samples in the first round. This is a
shared host; the observed ranges show variability, not confidence intervals.

| Fields per row | Fixture | Before ms, median [min–max] | After ms, median [min–max] | Median ratio |
| --- | --- | --- | --- | --- |
| 1 | Plain | 0.668 [0.659–0.732] | 0.206 [0.191–0.228] | 3.24× |
| 8 | Plain | 5.289 [5.096–6.340] | 1.268 [1.150–1.412] | 4.17× |
| 64 | Plain | 41.941 [40.895–42.575] | 9.910 [9.218–21.454] | 4.23× |
| 1 | Quoted Unicode | 2.136 [2.106–2.332] | 0.517 [0.493–0.583] | 4.13× |
| 8 | Quoted Unicode | 17.592 [17.211–25.530] | 3.862 [3.717–4.133] | 4.56× |
| 64 | Quoted Unicode | 137.107 [136.153–141.051] | 30.508 [29.665–31.271] | 4.49× |
| 1 | Large quoted Unicode | 34.917 [34.211–40.942] | 21.685 [21.230–22.737] | 1.61× |
| 8 | Large quoted Unicode | 288.481 [283.192–315.826] | 175.099 [172.114–185.593] | 1.65× |
| 64 | Large quoted Unicode | 2331.953 [2255.445–2406.398] | 1414.346 [1393.176–1720.227] | 1.65× |

Plain fields are `plain value`; quoted fields include Ukrainian text, quotes,
comma, CR and LF. The large field repeats `довгий текст, "rust" ` 64 times.

Allocation counting uses a separate process with `stats_alloc` 0.1.10:

| Eight-field row | Before allocations / reallocations | After allocations / reallocations | Before allocated bytes | After allocated bytes |
| --- | --- | --- | --- | --- |
| Plain | 8 / 0 | 0 / 0 | 88 | 0 |
| Quoted Unicode | 16 / 24 | 0 / 0 | 624 | 0 |
| Large quoted Unicode | 16 / 24 | 0 / 0 | 67,600 | 0 |

Counts and bytes are normalized per row from 20,000 operations. All nine
workloads measured zero allocations and reallocations in the new row encoder.
These results exclude initialization and buffer growth. Flattening `Item`
values, header inference, pipeline futures/channels, and file I/O remain
outside this measurement. This is not a zero-copy claim or a claim of
allocation-free CSV export. End-to-end crawl throughput, peak/live memory
and foreign allocations were not measured.

## Reproduce

Run the project benchmark, or compile the dependency-free row benchmark
directly with the same measured settings:

```bash
cargo +1.92.0 bench --bench csv_rows --locked
rustc +1.92.0 --edition=2024 -C opt-level=3 -C lto=fat -C codegen-units=1 benches/csv_rows.rs -o /tmp/silkworm-csv-bench
/tmp/silkworm-csv-bench
python3 scripts/measure_seen_allocations.py --toolchain 1.92.0 --benchmark csv_rows
```

The Cargo benchmark inherits the project's release profile, including its
abort-on-panic setting; the timing table above uses the direct `rustc` command.

## Validation

- Existing baseline: 120 tests passed; regression reproduced `capacity overflow`.
- Updated default and no-default configurations: 124 tests each passed.
- Updated all-features configuration: 125 tests passed.
- CSV differential test: all 2,801 fields of length 0–4 over seven tokens,
  including multibyte Unicode, quotes, delimiters and newlines, match the
  previous encoder. Separate tests check multi-field rows, append behavior
  and empty rows.
- Formatting, all-target checking, all-feature Clippy with `-D warnings`,
  and offline examples passed on Rust 1.92.0.

Native dependency checks used the existing CMake/libclang installation and
`BINDGEN_EXTRA_CLANG_ARGS=-I/usr/lib/gcc/x86_64-linux-gnu/13/include`.
No dependency or toolchain changes were made to the repository.
