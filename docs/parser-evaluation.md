# Parser replacement investigation (#17)

## Decision

Keep `scraper` as the production backend for now. `scraper-rust` uses
`rustedbytes-tl`, but its published Rust backend is not a behavior-compatible
replacement for silkworm's current parser. This investigation adds an isolated,
reproducible compatibility audit and Criterion comparison, without changing
silkworm's dependencies, public API or response behavior.

Sources checked on 2026-10-08:

- [scraper-rust on PyPI](https://pypi.org/project/scraper-rust/)
- [scraper-rs manifest](https://github.com/RustedBytes/scraper-rs/blob/master/Cargo.toml): `rustedbytes-tl` 0.2.0 with `std`; XPath separately uses `xee-xpath`.
- [rustedbytes-tl 0.2.0](https://docs.rs/rustedbytes-tl/0.2.0/tl/)
- [scraper 0.27.0](https://docs.rs/scraper/0.27.0/scraper/)
- [silkworm's response implementation](../src/response.rs)

The comparison pins `scraper = 0.27.0` with `atomic` and `rustedbytes-tl = 0.2.0`
with `std`. The TL package's Rust crate name is `tl`. Later backend releases may
change these findings; rerun the audit when updating the pinned versions.

## Demonstrated differences

The catalog fixture contains three `li.item` nodes, each with an `a[href]` child.
`None` below means the query was rejected; zero means a parsed query returned no
matches. These observations come from the published Rust APIs, not from Python
binding latency or the wrapper's marketing claims.

| Query or operation | scraper 0.27.0 | rustedbytes-tl 0.2.0 |
| --- | --- | --- |
| `li`, `.item`, `a[href]` | 3 matches | 3 matches |
| `li a`, `li > a`, `.item a` | 3 matches | 0 matches |
| `li:first-child` | 1 match | 0 matches |
| `li:nth-child(2)` | 1 match | Rejected |
| `li:not(.missing)`, `li:has(a)` | 3 matches | Rejected |
| `li + li` | 2 matches | 0 matches |
| `li ~ li` | 2 matches | Rejected |
| `a[href^='/items/']` | 3 matches | 3 matches |
| `table > tbody > tr` on `<table><tr><td>Cell</td></tr></table>` | 1 match: implied tbody | 0 matches |
| Text of `<p>A &amp; B&nbsp;C</p>` | `A & B` then U+00A0 then `C` | Literal `A &amp; B&nbsp;C` |

A fallback triggered only when TL rejects a selector is insufficient: several
queries are accepted and silently return an incompatible empty result. Simple
well-formed fixtures passing does not establish compatibility on web HTML.

## API and ownership work needed

`HtmlResponse` and `HtmlElement` expose `select_with`/`select_first_with` accepting
`&scraper::Selector`. Removing that dependency changes public type identity.
An adapter can serialize the existing selector through its `ToCss` implementation,
but keeping that API still requires the scraper type/dependency and does not solve
selector or DOM semantics.

The current response owns its source and caches an owned `scraper::Html` in
`OnceLock`. TL's safe `parse(&str, ...)` returns a DOM borrowing that source.
Storing both requires a safe owner/borrow abstraction. In 0.2.0, the documented
`parse_owned(String, ...)` constructor is `unsafe`; directly using it conflicts
with silkworm's `forbid(unsafe_code)` rule. This is an integration constraint,
not a finding of unsoundness in TL. Do not leak strings or extend lifetimes to
work around it. Also validate Send/Sync, clone/cache reset and cleanup behavior.

Element text, attributes, serialization and nested selection need adapters and
differential fixtures. Preserve malformed HTML recovery, entities, namespaces,
document order and HTML size limits. XPath remains a separate xee-xpath pipeline;
switching the CSS DOM alone does not replace it or establish XPath speedups.

## Initial timing evidence

Synthetic 200-row catalog, 10,830 UTF-8 input bytes. The common selector is
`a[href]` and both parsers return exactly 200 matches before timing.

| Operation | Time estimate | 95% confidence interval |
| --- | --- | --- |
| scraper parse + DOM drop | 274.29 us | 273.24–275.51 us |
| TL parse + DOM drop | 58.697 us | 57.598–59.880 us |
| scraper warm string query + count | 6.5415 us | 6.4052–6.7428 us |
| TL warm string query + count | 5.1603 us | 5.1255–5.1977 us |
| scraper warm compiled query + count | 5.9597 us | 5.9110–6.0141 us |

One exploratory run on a shared Linux x86_64 host (kernel 6.18.44, AMD EPYC 9V74),
Rust 1.99.0 / LLVM 23.1.1, default system allocator, no custom RUSTFLAGS. Profile:
opt-level 3, fat LTO, one codegen unit. Base silkworm revision:
`656f065e4a43d93a8c678f82a9aad2a6144996ab`; independent dependency versions are in
[the evaluation lockfile](../tools/parser-evaluation/Cargo.lock).

Criterion 0.8.2 defaults: 100 samples, 3 s warm-up, 5 s measurement, 95% intervals.
Outliers were retained; this run is not independently repeated or an acceptance
benchmark. The differences in parsing semantics mean parse-time ratios are not
an equivalent-work speedup or a whole-crawler improvement. Warm queries exclude
initial DOM parsing, both string-query cases include query parsing, and the
compiled scraper case excludes it. Output ownership/serialization, decoding,
networking, async scheduling, allocation counts and peak memory are not measured.
No zero-copy or zero-allocation conclusion follows from these timings.

## Reproduce

The evaluation crate is independent of the production package and native TLS.
It builds on Rust 1.92. Its tests cover common selectors and demonstrated
combinator/table differences. The executable prints the broader audit matrix.
Fixtures use no randomness or network requests. Criterion covers 10, 200 and
2,000 rows; all 15 cases were smoke-tested, and only the five 200-row cases were
fully measured in the initial run. Fixture creation and assertions are outside
measurement; `iter` includes output destruction.

```bash
cargo run --locked --release --manifest-path tools/parser-evaluation/Cargo.toml
cargo test --locked --release --manifest-path tools/parser-evaluation/Cargo.toml
cargo bench --locked --manifest-path tools/parser-evaluation/Cargo.toml --bench parsers -- --test
cargo bench --locked --manifest-path tools/parser-evaluation/Cargo.toml --bench parsers -- '/200$' --noplot --save-baseline UNIQUE_NAME
```

Use a fresh baseline name. The initial name was
`investigation-656f065-20261008`. Keep raw Criterion estimates/samples and logs
outside tracked source; do not commit target directories. Repeat comparisons on
an idle controlled host before making performance decisions.

## Conditions for revisiting the decision

1. Fix or explicitly restrict TL combinators/pseudo-classes and expand differential
   tests across a real HTML corpus. Accepted queries must return compatible matches.
2. Implement entity decoding, HTML tree recovery and serialization semantics, or
   define a separately opted-in API with a documented narrower contract.
3. Resolve owned DOM caching through a safe abstraction and test threading/lifecycles.
4. Benchmark the complete adapters, first/warm/nested selections and owned outputs;
   measure allocations separately. Include wrong/unsupported queries and malformed HTML.
5. If scraper's public selector types are removed, treat that as an explicit API
   migration rather than an invisible dependency substitution.
