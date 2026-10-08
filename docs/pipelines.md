# Pipelines

Pipelines consume scraped items and write them to files or custom handlers. Each
pipeline follows an async lifecycle: `open`, `process_item`, and `close`.
The engine passes each item through pipelines sequentially in registration
order; the returned item is input to the next pipeline. A pipeline error stops
processing that item. With `fail_fast` disabled it is logged and crawling
continues; enabling `fail_fast` stops the crawl.

## File Writer Ownership and Cancellation

JSONL, CSV and XML pipelines each use one writer task with a bounded command
queue of 16 items. The task owns the file, CSV header state and a reusable
record buffer. Concurrent `process_item` calls are serialized in command
admission order; records are not interleaved and CSV writes its header once.

Cancelling a call before its command is admitted does not submit the item.
Once admitted, the writer finishes the record even if the caller stops waiting.
`close` stops accepting new commands, drains admitted records, writes the XML
footer when applicable, and flushes the file. Cancelling `close` does not cancel
this work. Reopening is rejected until the previous writer finishes closing;
calling `open` on an already open pipeline also fails before touching the file.

An I/O failure is returned to the waiting caller and retained for subsequent
writes and `close`; no further records are written to a potentially partial
document. Errors are logged even when a caller has been cancelled. A successful
`process_item` acknowledges a buffered write; `close` flushes it, but does not
perform an `fsync` or guarantee durability after a process crash. The writer
requires the Tokio runtime to remain alive to finish accepted commands.

## ItemPipeline Trait

The `ItemPipeline` trait defines the pipeline interface and is used by the
engine after each item is emitted by a spider callback.

Code:
- Pipeline trait: `../src/pipelines.rs`
- Engine integration: `../src/engine.rs`

```rust
use std::sync::Arc;
use silkworm::{Item, ItemPipeline, PipelineFuture, SilkwormResult, Spider};

struct CustomPipeline;

impl<S: Spider> ItemPipeline<S> for CustomPipeline {
    fn open<'a>(&'a self, _spider: Arc<S>) -> PipelineFuture<'a, SilkwormResult<()>> {
        Box::pin(async move { Ok(()) })
    }

    fn close<'a>(&'a self, _spider: Arc<S>) -> PipelineFuture<'a, SilkwormResult<()>> {
        Box::pin(async move { Ok(()) })
    }

    fn process_item<'a>(
        &'a self,
        item: Item,
        _spider: Arc<S>,
    ) -> PipelineFuture<'a, SilkwormResult<Item>> {
        Box::pin(async move { Ok(item) })
    }
}
```

## Built-In Pipelines

### CallbackPipeline

Wraps an async (or sync) callback for custom per-item handling.

Code:
- `CallbackPipeline`: `../src/pipelines.rs`

### JsonLinesPipeline

Writes each item as a JSON line, appending to the target file. It creates
parent directories if needed.

Code:
- `JsonLinesPipeline`: `../src/pipelines.rs`

### CsvPipeline

Flattens nested items into a flat row format and writes CSV output. Field
names can be provided or inferred from the first item. Opening CSV truncates
an existing target file; later items do not expand the inferred field list.

Flattening rules:
- Object keys are flattened with underscore separators.
- Arrays are joined by commas.
- Scalar values are stringified.

Code:
- `CsvPipeline`: `../src/pipelines.rs`

### XmlPipeline

Writes items as nested XML. The pipeline writes a document header on open and
closes the root element on close. Tag names are sanitized (spaces and dashes
become underscores). Opening XML truncates an existing target file.

Code:
- `XmlPipeline`: `../src/pipelines.rs`

```rust
use silkworm::{CallbackPipeline, JsonLinesPipeline, RunConfig};

let config = RunConfig::<QuotesSpider>::new()
    .with_item_pipeline(JsonLinesPipeline::new("data/items.jl"))
    .with_item_pipeline(CallbackPipeline::from_sync(|item, _spider| Ok(item)));
```

## Save items as JSON Lines

Write scraped items to files or plug in your own callback:

```rust
use silkworm::{run_spider_with, JsonLinesPipeline, RunConfig};

let config = RunConfig::<QuotesSpider>::new().with_item_pipeline(JsonLinesPipeline::new("data/items.jl"));
run_spider_with(QuotesSpider, config)?;
```

The pipeline fragment replaces the body of the [README quickstart](../README.md#quick-start) synchronous `main` (return
`Ok(())` after the call). JSON Lines appends to an existing file.
