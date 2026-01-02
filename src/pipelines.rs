use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::Mutex as AsyncMutex;

use crate::errors::{SilkwormError, SilkwormResult};
use crate::logging::get_logger;
use crate::spider::Spider;
use crate::types::Item;

#[async_trait]
pub trait ItemPipeline<S: Spider>: Send + Sync {
    async fn open(&self, spider: Arc<S>) -> SilkwormResult<()>;
    async fn close(&self, spider: Arc<S>) -> SilkwormResult<()>;
    async fn process_item(&self, item: Item, spider: Arc<S>) -> SilkwormResult<Item>;
}

type PipelineFuture = Pin<Box<dyn Future<Output = SilkwormResult<Item>> + Send>>;

pub struct CallbackPipeline<S: Spider> {
    callback: Arc<dyn Fn(Item, Arc<S>) -> PipelineFuture + Send + Sync>,
    logger: crate::logging::Logger,
}

impl<S: Spider> CallbackPipeline<S> {
    pub fn new<F, Fut>(callback: F) -> Self
    where
        F: Fn(Item, Arc<S>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = SilkwormResult<Item>> + Send + 'static,
    {
        let cb: Arc<dyn Fn(Item, Arc<S>) -> PipelineFuture + Send + Sync> =
            Arc::new(move |item, spider| -> PipelineFuture { Box::pin(callback(item, spider)) });
        CallbackPipeline {
            callback: cb,
            logger: get_logger("CallbackPipeline", None),
        }
    }

    pub fn from_sync<F>(callback: F) -> Self
    where
        F: Fn(Item, Arc<S>) -> SilkwormResult<Item> + Send + Sync + 'static,
    {
        let cb: Arc<dyn Fn(Item, Arc<S>) -> PipelineFuture + Send + Sync> =
            Arc::new(move |item, spider| -> PipelineFuture {
                let result = callback(item, spider);
                Box::pin(async move { result })
            });
        CallbackPipeline {
            callback: cb,
            logger: get_logger("CallbackPipeline", None),
        }
    }
}

#[async_trait]
impl<S: Spider> ItemPipeline<S> for CallbackPipeline<S> {
    async fn open(&self, _spider: Arc<S>) -> SilkwormResult<()> {
        self.logger.info("Opened Callback pipeline", &[]);
        Ok(())
    }

    async fn close(&self, _spider: Arc<S>) -> SilkwormResult<()> {
        self.logger.info("Closed Callback pipeline", &[]);
        Ok(())
    }

    async fn process_item(&self, item: Item, spider: Arc<S>) -> SilkwormResult<Item> {
        (self.callback)(item, spider).await
    }
}

pub struct JsonLinesPipeline {
    path: PathBuf,
    state: AsyncMutex<JsonLinesState>,
    logger: crate::logging::Logger,
}

struct JsonLinesState {
    file: Option<BufWriter<tokio::fs::File>>,
}

impl JsonLinesPipeline {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        JsonLinesPipeline {
            path: path.into(),
            state: AsyncMutex::new(JsonLinesState { file: None }),
            logger: get_logger("JsonLinesPipeline", None),
        }
    }
}

#[async_trait]
impl<S: Spider> ItemPipeline<S> for JsonLinesPipeline {
    async fn open(&self, _spider: Arc<S>) -> SilkwormResult<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;
        let mut guard = self.state.lock().await;
        guard.file = Some(BufWriter::new(file));
        self.logger.info(
            "Opened JSON Lines pipeline",
            &[("path", self.path.display().to_string())],
        );
        Ok(())
    }

    async fn close(&self, _spider: Arc<S>) -> SilkwormResult<()> {
        let mut guard = self.state.lock().await;
        if let Some(file) = guard.file.as_mut() {
            file.flush().await?;
        }
        guard.file = None;
        self.logger.info(
            "Closed JSON Lines pipeline",
            &[("path", self.path.display().to_string())],
        );
        Ok(())
    }

    async fn process_item(&self, item: Item, _spider: Arc<S>) -> SilkwormResult<Item> {
        let line = serde_json::to_string(&item)
            .map_err(|err| SilkwormError::Pipeline(format!("JSON encode failed: {err}")))?;
        let mut guard = self.state.lock().await;
        let file = guard
            .file
            .as_mut()
            .ok_or_else(|| SilkwormError::Pipeline("JsonLinesPipeline not opened".to_string()))?;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
        Ok(item)
    }
}

pub struct CsvPipeline {
    path: PathBuf,
    state: AsyncMutex<CsvState>,
    logger: crate::logging::Logger,
}

struct CsvState {
    file: Option<BufWriter<tokio::fs::File>>,
    fieldnames: Option<Vec<String>>,
    header_written: bool,
}

impl CsvPipeline {
    pub fn new(path: impl Into<PathBuf>, fieldnames: Option<Vec<String>>) -> Self {
        CsvPipeline {
            path: path.into(),
            state: AsyncMutex::new(CsvState {
                file: None,
                fieldnames,
                header_written: false,
            }),
            logger: get_logger("CsvPipeline", None),
        }
    }
}

#[async_trait]
impl<S: Spider> ItemPipeline<S> for CsvPipeline {
    async fn open(&self, _spider: Arc<S>) -> SilkwormResult<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.path)
            .await?;
        let mut guard = self.state.lock().await;
        guard.file = Some(BufWriter::new(file));
        guard.header_written = false;
        self.logger.info(
            "Opened CSV pipeline",
            &[("path", self.path.display().to_string())],
        );
        Ok(())
    }

    async fn close(&self, _spider: Arc<S>) -> SilkwormResult<()> {
        let mut guard = self.state.lock().await;
        if let Some(file) = guard.file.as_mut() {
            file.flush().await?;
        }
        guard.file = None;
        self.logger.info(
            "Closed CSV pipeline",
            &[("path", self.path.display().to_string())],
        );
        Ok(())
    }

    async fn process_item(&self, item: Item, _spider: Arc<S>) -> SilkwormResult<Item> {
        let flattened = flatten_item(&item);
        let mut guard = self.state.lock().await;
        if guard.fieldnames.is_none() {
            guard.fieldnames = Some(flattened.keys().cloned().collect());
        }
        let (header, row, write_header) = {
            let fieldnames = guard.fieldnames.as_ref().ok_or_else(|| {
                SilkwormError::Pipeline("CsvPipeline fieldnames missing".to_string())
            })?;
            let header = if !guard.header_written {
                Some(
                    fieldnames
                        .iter()
                        .map(|name| csv_escape(name))
                        .collect::<Vec<_>>()
                        .join(","),
                )
            } else {
                None
            };
            let row = fieldnames
                .iter()
                .map(|name| flattened.get(name).cloned().unwrap_or_default())
                .map(|value| csv_escape(&value))
                .collect::<Vec<_>>()
                .join(",");
            let write_header = !guard.header_written;
            (header, row, write_header)
        };

        if write_header {
            if let Some(header) = header.as_ref() {
                {
                    let file = guard.file.as_mut().ok_or_else(|| {
                        SilkwormError::Pipeline("CsvPipeline not opened".to_string())
                    })?;
                    file.write_all(header.as_bytes()).await?;
                    file.write_all(b"\n").await?;
                }
            }
            guard.header_written = true;
        }

        let file = guard
            .file
            .as_mut()
            .ok_or_else(|| SilkwormError::Pipeline("CsvPipeline not opened".to_string()))?;
        file.write_all(row.as_bytes()).await?;
        file.write_all(b"\n").await?;
        Ok(item)
    }
}

pub struct XmlPipeline {
    path: PathBuf,
    root_element: String,
    item_element: String,
    file: AsyncMutex<Option<BufWriter<tokio::fs::File>>>,
    logger: crate::logging::Logger,
}

impl XmlPipeline {
    pub fn new(path: impl Into<PathBuf>, root_element: &str, item_element: &str) -> Self {
        XmlPipeline {
            path: path.into(),
            root_element: root_element.to_string(),
            item_element: item_element.to_string(),
            file: AsyncMutex::new(None),
            logger: get_logger("XmlPipeline", None),
        }
    }
}

#[async_trait]
impl<S: Spider> ItemPipeline<S> for XmlPipeline {
    async fn open(&self, _spider: Arc<S>) -> SilkwormResult<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.path)
            .await?;
        let mut guard = self.file.lock().await;
        let mut file = BufWriter::new(file);
        let header = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<{}>\n",
            sanitize_tag(&self.root_element)
        );
        file.write_all(header.as_bytes()).await?;
        *guard = Some(file);
        self.logger.info(
            "Opened XML pipeline",
            &[("path", self.path.display().to_string())],
        );
        Ok(())
    }

    async fn close(&self, _spider: Arc<S>) -> SilkwormResult<()> {
        let mut guard = self.file.lock().await;
        if let Some(file) = guard.as_mut() {
            let footer = format!("</{}>\n", sanitize_tag(&self.root_element));
            file.write_all(footer.as_bytes()).await?;
            file.flush().await?;
        }
        *guard = None;
        self.logger.info(
            "Closed XML pipeline",
            &[("path", self.path.display().to_string())],
        );
        Ok(())
    }

    async fn process_item(&self, item: Item, _spider: Arc<S>) -> SilkwormResult<Item> {
        let mut guard = self.file.lock().await;
        let file = guard
            .as_mut()
            .ok_or_else(|| SilkwormError::Pipeline("XmlPipeline not opened".to_string()))?;
        let xml = build_xml(&self.item_element, &item, 1);
        file.write_all(xml.as_bytes()).await?;
        Ok(item)
    }
}

fn flatten_item(item: &Item) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    match item {
        Item::Object(map) => {
            for (key, value) in map {
                flatten_value(key, value, &mut out);
            }
        }
        _ => {
            out.insert("item".to_string(), scalar_to_string(item));
        }
    }
    out
}

fn flatten_value(prefix: &str, value: &Item, out: &mut BTreeMap<String, String>) {
    match value {
        Item::Object(map) => {
            for (key, nested) in map {
                let next = format!("{}_{}", prefix, key);
                flatten_value(&next, nested, out);
            }
        }
        Item::Array(items) => {
            let joined = items
                .iter()
                .map(scalar_to_string)
                .collect::<Vec<_>>()
                .join(",");
            out.insert(prefix.to_string(), joined);
        }
        _ => {
            out.insert(prefix.to_string(), scalar_to_string(value));
        }
    }
}

fn scalar_to_string(value: &Item) -> String {
    match value {
        Item::Null => String::new(),
        Item::Bool(v) => v.to_string(),
        Item::Number(v) => v.to_string(),
        Item::String(v) => v.clone(),
        Item::Array(_) | Item::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn csv_escape(value: &str) -> String {
    let needs_quotes =
        value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r');
    if needs_quotes {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn build_xml(tag: &str, value: &Item, depth: usize) -> String {
    let tag = sanitize_tag(tag);
    let indent = "  ".repeat(depth);
    match value {
        Item::Object(map) => {
            let mut out = format!("{indent}<{tag}>\n");
            for (key, nested) in map {
                out.push_str(&build_xml(key, nested, depth + 1));
            }
            out.push_str(&format!("{indent}</{tag}>\n"));
            out
        }
        Item::Array(items) => {
            let mut out = format!("{indent}<{tag}>\n");
            for item in items {
                out.push_str(&build_xml("item", item, depth + 1));
            }
            out.push_str(&format!("{indent}</{tag}>\n"));
            out
        }
        _ => {
            let text = escape_xml(&scalar_to_string(value));
            format!("{indent}<{tag}>{text}</{tag}>\n")
        }
    }
}

fn sanitize_tag(tag: &str) -> String {
    let mut cleaned = tag.replace([' ', '-'], "_");
    if cleaned.is_empty() {
        cleaned = "item".to_string();
    }
    cleaned
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::{build_xml, csv_escape, flatten_item, sanitize_tag};
    use crate::types::Item;

    #[test]
    fn flatten_item_flattens_nested_objects_and_arrays() {
        let mut inner = serde_json::Map::new();
        inner.insert("name".to_string(), Item::from("Ada"));
        inner.insert("age".to_string(), Item::from(42));

        let mut outer = serde_json::Map::new();
        outer.insert("user".to_string(), Item::Object(inner));
        outer.insert(
            "tags".to_string(),
            Item::Array(vec![Item::from("a"), Item::from("b")]),
        );

        let flat = flatten_item(&Item::Object(outer));
        assert_eq!(flat.get("user_name").map(String::as_str), Some("Ada"));
        assert_eq!(flat.get("user_age").map(String::as_str), Some("42"));
        assert_eq!(flat.get("tags").map(String::as_str), Some("a,b"));
    }

    #[test]
    fn csv_escape_quotes_when_needed() {
        assert_eq!(csv_escape("plain"), "plain");
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
        assert_eq!(csv_escape("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn sanitize_tag_replaces_invalid_chars() {
        assert_eq!(sanitize_tag("my tag-name"), "my_tag_name");
        assert_eq!(sanitize_tag(""), "item");
    }

    #[test]
    fn build_xml_escapes_text() {
        let xml = build_xml("item", &Item::from("a&b"), 1);
        assert_eq!(xml, "  <item>a&amp;b</item>\n");
    }
}
