# Installation and features

The current repository requires Rust **1.92 or newer** (edition 2024).
The package is `silkworm-rs`; Rust imports use `silkworm`.
The commands below install the published crate. To use the current checkout,
see [development and verification](development.md).
Start with the complete [README quickstart](../README.md#quick-start).

Create a binary project with `cargo new my-spider`, then run these commands
inside it. Native TLS dependencies require a C/C++ toolchain, CMake and libclang
with its headers available to bindgen (see the development guide).

```bash
cargo add silkworm-rs
```

If you want to use the async API directly (instead of `run_spider`), add [Tokio](https://crates.io/crates/tokio):

```bash
cargo add tokio --features rt-multi-thread,macros
```

The [README quickstart](../README.md#quick-start) and guide snippets also use [`serde_json`](https://crates.io/crates/serde_json) for convenience:

```bash
cargo add serde_json
```

Enable XPath explicitly when needed:

```bash
cargo add silkworm-rs --features xpath
```

The default `scraper-atomic` feature caches parsed HTML. Without default
features, CSS remains available but documents are parsed per selection.
`cli-examples` enables clap-based repository examples; it is not required by
application spiders.

Enable automatic detection of undeclared non-UTF-8 text with
`cargo add silkworm-rs --features charset-detection`. This optional feature uses
[`charset-norm`](https://docs.rs/charset-norm/) and requires **Rust 1.98 or newer**;
default builds retain Rust 1.92 support. BOM, HTTP charset and HTML/XML declarations
remain authoritative, and valid UTF-8 bypasses statistical detection. See
[response decoding](core-concepts.md#response-and-htmlresponse).

The optional `tl-parser` feature adds `TlDocument`, `TlElement` and
`HtmlResponse::tl_document()` using rustedbytes-tl 0.3.0. It offers explicit
lightweight parsing; existing scraper APIs keep their behavior. See
[TL integration and compatibility limits](parser-evaluation.md).

Tip: `use silkworm::prelude::*;` for the most common types.
