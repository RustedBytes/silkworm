# Pipelines

Pipelines consume scraped items and write them to files or custom handlers. Each
pipeline follows an async lifecycle: `open`, `process_item`, and `close`.

## ItemPipeline Trait

The `ItemPipeline` trait defines the pipeline interface and is used by the
engine after each item is emitted by a spider callback.

Code:
- Pipeline trait: `../src/pipelines.rs`
- Engine integration: `../src/engine.rs`

```rust
#[async_trait::async_trait]
impl<S: Spider> ItemPipeline<S> for CustomPipeline {
    async fn open(&self, _spider: Arc<S>) -> SilkwormResult<()> { Ok(()) }
    async fn close(&self, _spider: Arc<S>) -> SilkwormResult<()> { Ok(()) }
    async fn process_item(&self, item: Item, _spider: Arc<S>) -> SilkwormResult<Item> {
        Ok(item)
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
names can be provided or inferred from the first item.

Flattening rules:
- Object keys are flattened with underscore separators.
- Arrays are joined by commas.
- Scalar values are stringified.

Code:
- `CsvPipeline`: `../src/pipelines.rs`

### XmlPipeline

Writes items as nested XML. The pipeline writes a document header on open and
closes the root element on close. Tag names are sanitized (spaces and dashes
become underscores).

Code:
- `XmlPipeline`: `../src/pipelines.rs`

```rust
use silkworm::{CallbackPipeline, JsonLinesPipeline, RunConfig};

let config = RunConfig::new()
    .with_item_pipeline(JsonLinesPipeline::new("data/items.jl"))
    .with_item_pipeline(CallbackPipeline::from_sync(|item, _spider| Ok(item)));
```
