# API and extraction baseline

Source: src/spider.rs, src/request.rs, src/response.rs, src/types.rs,
examples/quotes_spider.rs and docs/core-concepts.md (silkworm-rs 0.2.1).
Re-read the resolved source before relying on these signatures.

- Depend on package `silkworm-rs`; import crate `silkworm`.
- Spider is Send + Sync + 'static. Its async parse receives HtmlResponse<Self>
  and returns SpiderResult<Self> (a result containing output values).
- `Request::get` accepts owned URLs. Use builders for headers, params, timeout,
  proxy and callbacks; inspect src/request.rs for the current callback signature.
  A callback replaces Spider::parse for that request. Prefer named handlers
  and retain typed spider state rather than encoding everything into metadata.
- `response.select(css)?` returns elements. Use element text_from/attr helpers
  for extraction; missing required data should be explicitly rejected or counted.
- `response.follow_url(href)` joins relative URLs. Follow helpers do not define
  your origin policy: check the resulting URL before returning it.
- `item_from(serializable)?` produces an Item (serde_json::Value).
- Request fingerprints use method and canonical URL with merged query params;
  body/headers are not part of that identity. Review collisions for POST/API
  crawls. dont_filter bypasses deduplication; avoid applying it to all pagination.
- Higher request priority runs first, equal priority is FIFO.
- CSS is available by default. XPath requires `xpath`; propagate disabled-feature
  errors rather than interpreting them as an empty document. Precompile selectors
  when repeatedly used, using select_with and the resolved XPath API.
- `tl-parser` is an opt-in alternative, not a replacement for scraper semantics.
  Measure extraction parity before adopting it.
- UtilityFetcher reuses a configured HTTP client; fetch_*_with constructs one
  per call. Prefer a reused fetcher for repeated custom utility requests.
