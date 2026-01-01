use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
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
    file: AsyncMutex<Option<tokio::fs::File>>,
    logger: crate::logging::Logger,
}

impl JsonLinesPipeline {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        JsonLinesPipeline {
            path: path.into(),
            file: AsyncMutex::new(None),
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
        let mut guard = self.file.lock().await;
        *guard = Some(file);
        self.logger.info(
            "Opened JSON Lines pipeline",
            &[("path", self.path.display().to_string())],
        );
        Ok(())
    }

    async fn close(&self, _spider: Arc<S>) -> SilkwormResult<()> {
        let mut guard = self.file.lock().await;
        *guard = None;
        self.logger.info(
            "Closed JSON Lines pipeline",
            &[("path", self.path.display().to_string())],
        );
        Ok(())
    }

    async fn process_item(&self, item: Item, _spider: Arc<S>) -> SilkwormResult<Item> {
        let line = serde_json::to_string(&item)
            .map_err(|err| SilkwormError::Pipeline(format!("JSON encode failed: {err}")))?;
        let mut guard = self.file.lock().await;
        let file = guard
            .as_mut()
            .ok_or_else(|| SilkwormError::Pipeline("JsonLinesPipeline not opened".to_string()))?;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
        file.flush().await?;
        Ok(item)
    }
}

pub struct CsvPipeline {
    path: PathBuf,
    fieldnames: AsyncMutex<Option<Vec<String>>>,
    header_written: AsyncMutex<bool>,
    file: AsyncMutex<Option<tokio::fs::File>>,
    logger: crate::logging::Logger,
}

impl CsvPipeline {
    pub fn new(path: impl Into<PathBuf>, fieldnames: Option<Vec<String>>) -> Self {
        CsvPipeline {
            path: path.into(),
            fieldnames: AsyncMutex::new(fieldnames),
            header_written: AsyncMutex::new(false),
            file: AsyncMutex::new(None),
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
        let mut guard = self.file.lock().await;
        *guard = Some(file);
        self.logger.info(
            "Opened CSV pipeline",
            &[("path", self.path.display().to_string())],
        );
        Ok(())
    }

    async fn close(&self, _spider: Arc<S>) -> SilkwormResult<()> {
        let mut guard = self.file.lock().await;
        *guard = None;
        self.logger.info(
            "Closed CSV pipeline",
            &[("path", self.path.display().to_string())],
        );
        Ok(())
    }

    async fn process_item(&self, item: Item, _spider: Arc<S>) -> SilkwormResult<Item> {
        let flattened = flatten_item(&item);
        let mut fieldnames_guard = self.fieldnames.lock().await;
        let fieldnames = if let Some(names) = fieldnames_guard.as_ref() {
            names.clone()
        } else {
            let names: Vec<String> = flattened.keys().cloned().collect();
            *fieldnames_guard = Some(names.clone());
            names
        };
        drop(fieldnames_guard);

        let mut guard = self.file.lock().await;
        let file = guard
            .as_mut()
            .ok_or_else(|| SilkwormError::Pipeline("CsvPipeline not opened".to_string()))?;
        let mut header_guard = self.header_written.lock().await;
        if !*header_guard {
            let header = fieldnames
                .iter()
                .map(|name| csv_escape(name))
                .collect::<Vec<_>>()
                .join(",");
            file.write_all(header.as_bytes()).await?;
            file.write_all(b"\n").await?;
            *header_guard = true;
        }

        let row = fieldnames
            .iter()
            .map(|name| flattened.get(name).cloned().unwrap_or_default())
            .map(|value| csv_escape(&value))
            .collect::<Vec<_>>()
            .join(",");
        file.write_all(row.as_bytes()).await?;
        file.write_all(b"\n").await?;
        file.flush().await?;
        Ok(item)
    }
}

pub struct XmlPipeline {
    path: PathBuf,
    root_element: String,
    item_element: String,
    file: AsyncMutex<Option<tokio::fs::File>>,
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
        let mut file = file;
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
        file.flush().await?;
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
                .map(|value| scalar_to_string(value))
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
    let mut cleaned = tag.replace(' ', "_").replace('-', "_");
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
