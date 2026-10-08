# Parser replacement investigation (#17)

## Current integration: rustedbytes-tl 0.3.0

The published `0.3.0` backend resolves the catalog combinator/pseudo-class,
decoded-text and safe owned-DOM blockers observed in the original investigation.
It can now be used explicitly through silkworm's optional `tl-parser` feature:

```toml
silkworm-rs = { git = "https://github.com/RustedBytes/silkworm", features = ["tl-parser"] }
```

This integration is in the current Git source, not the previously published
silkworm 0.2.1 package. Until this PR is merged, select its `investigate/tl-parser` branch or exact
commit as well. Commit Cargo.lock to pin the selected Git revision.

With `use silkworm::*;`, an HTML response can produce an independent TL snapshot:

```rust
#[cfg(feature = "tl-parser")]
{
    let document = response.tl_document()?;
    for element in document.select("a[href]")? {
        let text = element.text();
        let href = element.raw_attr("href");
        println!("{text}: {href:?}");
    }
}
```

Or use `TlDocument::parse(String)` directly. The snapshot owns input through
TL's safe `VDomGuard::parse`; no unsafe code or lifetime extension is added to
silkworm. Keep a document for repeated queries. Elements and iterators borrow
it; moving the document to a worker is supported, but elements cannot outlive
it. `TlElement::select` queries descendants in the original tree without
serialization/reparsing. String selectors are parsed per call; structural
indexes and decoded aggregate text can allocate. This is not a zero-allocation
claim.

`HtmlResponse::tl_document()` copies the cached decoded, byte-limited source and
parses it once per call. Its result is independent of response mutation/drop.
This preserves decoding and size limits, including the zero limit. It does not
populate or replace the existing scraper DOM cache. Store the returned document
instead of repeatedly calling the method.

`select`/`select_first` preserve rejected-query errors; valid no-match queries
return empty results. `text` decodes character references; `raw_attr` explicitly
retains raw attribute values, with an empty string for present valueless
attributes. TL HTML serialization is not scraper's normalized serialization.

The production default remains scraper. Existing `scraper::Selector` APIs,
`HtmlElement`, utility `fetch_document` return types and XPath are unchanged.
There is no automatic fallback from TL to scraper. CSS escapes, namespace
selectors, attribute case flags and relative `:has(> a)` remain unsupported.
HTML5 recovery is still missing: TL does not insert implied `tbody`, perform
foster parenting or apply the adoption agency algorithm. These differences
prevent transparent replacement on arbitrary web HTML. Opt in only where the
narrower contract is acceptable; test real site fixtures before migrating a spider.

The independent evaluation now pins TL 0.3.0 with `entities` and scraper 0.27.0.
It checks agreement on the catalog, decoded mixed text and script raw text, and
retains regressions for implied table structure, raw attributes and unsupported
CSS. Its lockfile fixes the comparison dependencies. The old timing results
below belong to TL **0.2.0** and do not measure this adapter or TL 0.3.0.

## Historical investigation: rustedbytes-tl 0.2.0

The initial decision was to retain scraper because the published 0.2.0 backend
was incompatible. Sources checked on 2026-10-08:

- [scraper-rust on PyPI](https://pypi.org/project/scraper-rust/)
- [rustedbytes-tl 0.2.0](https://docs.rs/rustedbytes-tl/0.2.0/tl/)
- [rustedbytes-tl 0.3.0](https://docs.rs/rustedbytes-tl/0.3.0/tl/)
- [scraper 0.27.0](https://docs.rs/scraper/0.27.0/scraper/)
- [response implementation](../src/response.rs) and [TL adapter](../src/tl.rs)

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
`656f065e4a43d93a8c678f82a9aad2a6144996ab`; historical dependency versions are in
[the original evaluation lockfile](https://github.com/RustedBytes/silkworm/blob/7eeafb13602b25651de555b48402997ee50dcef6/tools/parser-evaluation/Cargo.lock).

Criterion 0.8.2 defaults: 100 samples, 3 s warm-up, 5 s measurement, 95% intervals.
Outliers were retained; this run is not independently repeated or an acceptance
benchmark. The differences in parsing semantics mean parse-time ratios are not
an equivalent-work speedup or a whole-crawler improvement. Warm queries exclude
initial DOM parsing, both string-query cases include query parsing, and the
compiled scraper case excludes it. Output ownership/serialization, decoding,
networking, async scheduling, allocation counts and peak memory are not measured.
No zero-copy or zero-allocation conclusion follows from these timings.

## Run the current 0.3.0 evaluation

These commands now test and benchmark TL 0.3.0. To reproduce the historical
0.2.0 timing table, check out the original PR commit
`7eeafb13602b25651de555b48402997ee50dcef6` and its lockfile instead.


The evaluation crate is independent of the production package and native TLS.
It builds on Rust 1.92. Its tests cover the current selector agreement and remaining
tree/attribute/CSS differences. The executable prints the broader audit matrix.
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

## Requirements for replacing the default backend

1. Expand differential tests across a real HTML corpus. Accepted queries must
   return compatible matches; the synthetic catalog alone is insufficient.
2. Implement HTML tree recovery, attribute decoding and serialization semantics, or
   define a separately opted-in API with a documented narrower contract.
3. Validate a default-backend cache strategy and response threading/lifecycles;
   the explicit adapter currently owns a separate safe snapshot.
4. Benchmark the complete adapters, first/warm/nested selections and owned outputs;
   measure allocations separately. Include wrong/unsupported queries and malformed HTML.
5. If scraper's public selector types are removed, treat that as an explicit API
   migration rather than an invisible dependency substitution.


## Integration verification

The adapter's tests cover decoded text and raw attributes, nested queries against
the original DOM, rejected versus empty queries, zero and byte-limited response
sources, HTTP charset decoding, response mutation/drop, Send/Sync and moving the
owner to a worker. A compile-fail doctest checks that an element cannot outlive
its document. These are synthetic regression fixtures, not a broad real-site
corpus or proof of HTML5 equivalence.

```bash
cargo test --locked --features tl-parser
cargo test --locked --no-default-features --features tl-parser
cargo test --locked --all-features
cargo clippy --locked --all-targets --features tl-parser -- -D warnings
```

The first two configurations run in the existing Rust 1.92 CI job; all features
need Rust 1.98 or newer due to `charset-detection`. The independent evaluation
workflow is filtered to its source, report and workflow changes. It runs tests,
Clippy and Criterion smoke checks without building the HTTP/TLS dependencies.
