use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use tokio::fs::OpenOptions;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::Mutex as AsyncMutex;

use crate::errors::{SilkwormError, SilkwormResult};
use crate::logging::get_logger;
use crate::spider::Spider;
use crate::types::Item;

pub type PipelineFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait ItemPipeline<S: Spider>: Send + Sync {
    fn open<'a>(&'a self, spider: Arc<S>) -> PipelineFuture<'a, SilkwormResult<()>>;
    fn close<'a>(&'a self, spider: Arc<S>) -> PipelineFuture<'a, SilkwormResult<()>>;
    fn process_item<'a>(
        &'a self,
        item: Item,
        spider: Arc<S>,
    ) -> PipelineFuture<'a, SilkwormResult<Item>>;
}

type PipelineCallbackFuture = Pin<Box<dyn Future<Output = SilkwormResult<Item>> + Send>>;

pub struct CallbackPipeline<S: Spider> {
    callback: Arc<dyn Fn(Item, Arc<S>) -> PipelineCallbackFuture + Send + Sync>,
    logger: crate::logging::Logger,
}

impl<S: Spider> CallbackPipeline<S> {
    #[must_use]
    pub fn new<F, Fut>(callback: F) -> Self
    where
        F: Fn(Item, Arc<S>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = SilkwormResult<Item>> + Send + 'static,
    {
        let cb: Arc<dyn Fn(Item, Arc<S>) -> PipelineCallbackFuture + Send + Sync> =
            Arc::new(move |item, spider| -> PipelineCallbackFuture {
                Box::pin(callback(item, spider))
            });
        CallbackPipeline {
            callback: cb,
            logger: get_logger("CallbackPipeline", None),
        }
    }

    #[must_use]
    pub fn from_sync<F>(callback: F) -> Self
    where
        F: Fn(Item, Arc<S>) -> SilkwormResult<Item> + Send + Sync + 'static,
    {
        let cb: Arc<dyn Fn(Item, Arc<S>) -> PipelineCallbackFuture + Send + Sync> =
            Arc::new(move |item, spider| -> PipelineCallbackFuture {
                let result = callback(item, spider);
                Box::pin(async move { result })
            });
        CallbackPipeline {
            callback: cb,
            logger: get_logger("CallbackPipeline", None),
        }
    }
}

impl<S: Spider> ItemPipeline<S> for CallbackPipeline<S> {
    fn open<'a>(&'a self, _spider: Arc<S>) -> PipelineFuture<'a, SilkwormResult<()>> {
        Box::pin(async move {
            self.logger.info("Opened Callback pipeline", &[]);
            Ok(())
        })
    }

    fn close<'a>(&'a self, _spider: Arc<S>) -> PipelineFuture<'a, SilkwormResult<()>> {
        Box::pin(async move {
            self.logger.info("Closed Callback pipeline", &[]);
            Ok(())
        })
    }

    fn process_item<'a>(
        &'a self,
        item: Item,
        spider: Arc<S>,
    ) -> PipelineFuture<'a, SilkwormResult<Item>> {
        Box::pin(async move { (self.callback)(item, spider).await })
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
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        JsonLinesPipeline {
            path: path.into(),
            state: AsyncMutex::new(JsonLinesState { file: None }),
            logger: get_logger("JsonLinesPipeline", None),
        }
    }
}

impl<S: Spider> ItemPipeline<S> for JsonLinesPipeline {
    fn open<'a>(&'a self, _spider: Arc<S>) -> PipelineFuture<'a, SilkwormResult<()>> {
        Box::pin(async move {
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
        })
    }

    fn close<'a>(&'a self, _spider: Arc<S>) -> PipelineFuture<'a, SilkwormResult<()>> {
        Box::pin(async move {
            let file = {
                let mut guard = self.state.lock().await;
                guard.file.take()
            };
            if let Some(mut file) = file {
                file.flush().await?;
            }
            self.logger.info(
                "Closed JSON Lines pipeline",
                &[("path", self.path.display().to_string())],
            );
            Ok(())
        })
    }

    fn process_item<'a>(
        &'a self,
        item: Item,
        _spider: Arc<S>,
    ) -> PipelineFuture<'a, SilkwormResult<Item>> {
        Box::pin(async move {
            let line = serde_json::to_string(&item)
                .map_err(|err| SilkwormError::Pipeline(format!("JSON encode failed: {err}")))?;
            let mut file = {
                let mut guard = self.state.lock().await;
                guard.file.take().ok_or_else(|| {
                    SilkwormError::Pipeline("JsonLinesPipeline not opened".to_string())
                })?
            };

            let write_result: SilkwormResult<()> = async {
                file.write_all(line.as_bytes()).await?;
                file.write_all(b"\n").await?;
                Ok(())
            }
            .await;

            let mut guard = self.state.lock().await;
            guard.file = Some(file);
            drop(guard);

            write_result?;
            Ok(item)
        })
    }
}

pub struct CsvPipeline {
    path: PathBuf,
    configured_fieldnames: Option<Vec<String>>,
    state: AsyncMutex<CsvState>,
    logger: crate::logging::Logger,
}

struct CsvState {
    file: Option<BufWriter<tokio::fs::File>>,
    fieldnames: Option<Vec<String>>,
    header_written: bool,
}

impl CsvPipeline {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, fieldnames: Option<Vec<String>>) -> Self {
        CsvPipeline {
            path: path.into(),
            configured_fieldnames: fieldnames.clone(),
            state: AsyncMutex::new(CsvState {
                file: None,
                fieldnames,
                header_written: false,
            }),
            logger: get_logger("CsvPipeline", None),
        }
    }
}

impl<S: Spider> ItemPipeline<S> for CsvPipeline {
    fn open<'a>(&'a self, _spider: Arc<S>) -> PipelineFuture<'a, SilkwormResult<()>> {
        Box::pin(async move {
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
            guard.fieldnames.clone_from(&self.configured_fieldnames);
            guard.header_written = false;
            self.logger.info(
                "Opened CSV pipeline",
                &[("path", self.path.display().to_string())],
            );
            Ok(())
        })
    }

    fn close<'a>(&'a self, _spider: Arc<S>) -> PipelineFuture<'a, SilkwormResult<()>> {
        Box::pin(async move {
            let file = {
                let mut guard = self.state.lock().await;
                guard.file.take()
            };
            if let Some(mut file) = file {
                file.flush().await?;
            }
            self.logger.info(
                "Closed CSV pipeline",
                &[("path", self.path.display().to_string())],
            );
            Ok(())
        })
    }

    fn process_item<'a>(
        &'a self,
        item: Item,
        _spider: Arc<S>,
    ) -> PipelineFuture<'a, SilkwormResult<Item>> {
        Box::pin(async move {
            let flattened = flatten_item(&item);
            let (mut file, fieldnames, header_written) = {
                let mut guard = self.state.lock().await;
                if guard.fieldnames.is_none() {
                    guard.fieldnames = Some(flattened.keys().cloned().collect());
                }
                let fieldnames = guard.fieldnames.clone().ok_or_else(|| {
                    SilkwormError::Pipeline("CsvPipeline fieldnames missing".to_string())
                })?;
                let file = guard
                    .file
                    .take()
                    .ok_or_else(|| SilkwormError::Pipeline("CsvPipeline not opened".to_string()))?;
                (file, fieldnames, guard.header_written)
            };

            let header = if header_written {
                None
            } else {
                Some(
                    fieldnames
                        .iter()
                        .map(|name| csv_escape(name))
                        .collect::<Vec<_>>()
                        .join(","),
                )
            };
            let row = fieldnames
                .iter()
                .map(|name| flattened.get(name).cloned().unwrap_or_default())
                .map(|value| csv_escape(&value))
                .collect::<Vec<_>>()
                .join(",");

            let wrote_header = header.is_some();
            let write_result: SilkwormResult<()> = async {
                if let Some(header) = header.as_ref() {
                    file.write_all(header.as_bytes()).await?;
                    file.write_all(b"\n").await?;
                }
                file.write_all(row.as_bytes()).await?;
                file.write_all(b"\n").await?;
                Ok(())
            }
            .await;

            let mut guard = self.state.lock().await;
            guard.file = Some(file);
            if write_result.is_ok() && wrote_header {
                guard.header_written = true;
            }
            drop(guard);

            write_result?;
            Ok(item)
        })
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
    #[must_use]
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

impl<S: Spider> ItemPipeline<S> for XmlPipeline {
    fn open<'a>(&'a self, _spider: Arc<S>) -> PipelineFuture<'a, SilkwormResult<()>> {
        Box::pin(async move {
            if let Some(parent) = self.path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&self.path)
                .await?;
            let mut file = BufWriter::new(file);
            let header = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<{}>\n",
                sanitize_tag(&self.root_element)
            );
            file.write_all(header.as_bytes()).await?;
            let mut guard = self.file.lock().await;
            *guard = Some(file);
            self.logger.info(
                "Opened XML pipeline",
                &[("path", self.path.display().to_string())],
            );
            Ok(())
        })
    }

    fn close<'a>(&'a self, _spider: Arc<S>) -> PipelineFuture<'a, SilkwormResult<()>> {
        Box::pin(async move {
            let file = {
                let mut guard = self.file.lock().await;
                guard.take()
            };
            if let Some(mut file) = file {
                let footer = format!("</{}>\n", sanitize_tag(&self.root_element));
                file.write_all(footer.as_bytes()).await?;
                file.flush().await?;
            }
            self.logger.info(
                "Closed XML pipeline",
                &[("path", self.path.display().to_string())],
            );
            Ok(())
        })
    }

    fn process_item<'a>(
        &'a self,
        item: Item,
        _spider: Arc<S>,
    ) -> PipelineFuture<'a, SilkwormResult<Item>> {
        Box::pin(async move {
            let mut file = {
                let mut guard = self.file.lock().await;
                guard
                    .take()
                    .ok_or_else(|| SilkwormError::Pipeline("XmlPipeline not opened".to_string()))?
            };
            let xml = build_xml(&self.item_element, &item, 1);
            let write_result: SilkwormResult<()> = async {
                file.write_all(xml.as_bytes()).await?;
                Ok(())
            }
            .await;
            let mut guard = self.file.lock().await;
            *guard = Some(file);
            drop(guard);

            write_result?;
            Ok(item)
        })
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
                let next = format!("{prefix}_{key}");
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
        // `serde_json::Value` already implements JSON rendering via Display.
        Item::Array(_) | Item::Object(_) => value.to_string(),
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
            let _ = writeln!(out, "{indent}</{tag}>");
            out
        }
        Item::Array(items) => {
            let mut out = format!("{indent}<{tag}>\n");
            for item in items {
                out.push_str(&build_xml("item", item, depth + 1));
            }
            let _ = writeln!(out, "{indent}</{tag}>");
            out
        }
        _ => {
            let text = escape_xml(&scalar_to_string(value));
            format!("{indent}<{tag}>{text}</{tag}>\n")
        }
    }
}

fn sanitize_tag(tag: &str) -> String {
    let mut cleaned = String::with_capacity(tag.len());
    for (idx, ch) in tag.chars().enumerate() {
        let valid = if idx == 0 {
            ch.is_ascii_alphabetic() || ch == '_'
        } else {
            ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | ':')
        };
        if valid {
            cleaned.push(ch);
        } else if !cleaned.ends_with('_') {
            cleaned.push('_');
        }
    }
    if cleaned.is_empty() {
        return "item".to_string();
    }
    if !cleaned
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
    {
        cleaned.insert(0, '_');
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
    use super::{CsvPipeline, ItemPipeline, build_xml, csv_escape, flatten_item, sanitize_tag};
    use crate::request::SpiderResult;
    use crate::response::HtmlResponse;
    use crate::spider::Spider;
    use crate::types::Item;
    use std::sync::Arc;

    struct TestSpider;

    impl Spider for TestSpider {
        fn name(&self) -> &str {
            "test"
        }

        async fn parse(&self, _response: HtmlResponse<Self>) -> SpiderResult<Self> {
            Ok(Vec::new())
        }
    }

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
        assert_eq!(sanitize_tag("9 bad<tag"), "_bad_tag");
    }

    #[test]
    fn build_xml_escapes_text() {
        let xml = build_xml("item", &Item::from("a&b"), 1);
        assert_eq!(xml, "  <item>a&amp;b</item>\n");
    }

    #[tokio::test]
    async fn csv_pipeline_resets_inferred_headers_between_open_calls() {
        let path = format!(
            "{}/silkworm_csv_reopen_{}_{}.csv",
            std::env::temp_dir().display(),
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        );
        let pipeline = CsvPipeline::new(&path, None);
        let spider = Arc::new(TestSpider);

        pipeline.open(spider.clone()).await.expect("open #1");
        pipeline
            .process_item(serde_json::json!({ "a": 1 }), spider.clone())
            .await
            .expect("item #1");
        pipeline.close(spider.clone()).await.expect("close #1");

        pipeline.open(spider.clone()).await.expect("open #2");
        pipeline
            .process_item(serde_json::json!({ "b": 2 }), spider.clone())
            .await
            .expect("item #2");
        pipeline.close(spider).await.expect("close #2");

        let content = std::fs::read_to_string(&path).expect("csv read");
        assert!(content.starts_with("b\n"));
        let _ = std::fs::remove_file(path);
    }
}
