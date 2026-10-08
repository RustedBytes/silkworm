use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

mod csv;
mod writer;
use writer::{FileWriter, RecordFormat};

use crate::errors::SilkwormResult;
use crate::logging::get_logger;
use crate::spider::Spider;
use crate::types::Item;

pub type PipelineFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait ItemPipeline<S: Spider>: Send + Sync {
    fn open(&self, spider: Arc<S>) -> PipelineFuture<'_, SilkwormResult<()>>;
    fn close(&self, spider: Arc<S>) -> PipelineFuture<'_, SilkwormResult<()>>;
    fn process_item(&self, item: Item, spider: Arc<S>) -> PipelineFuture<'_, SilkwormResult<Item>>;
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
    fn open(&self, _spider: Arc<S>) -> PipelineFuture<'_, SilkwormResult<()>> {
        Box::pin(async move {
            self.logger.info("Opened Callback pipeline", &[]);
            Ok(())
        })
    }

    fn close(&self, _spider: Arc<S>) -> PipelineFuture<'_, SilkwormResult<()>> {
        Box::pin(async move {
            self.logger.info("Closed Callback pipeline", &[]);
            Ok(())
        })
    }

    fn process_item(&self, item: Item, spider: Arc<S>) -> PipelineFuture<'_, SilkwormResult<Item>> {
        Box::pin(async move { (self.callback)(item, spider).await })
    }
}

pub struct JsonLinesPipeline {
    path: PathBuf,
    writer: FileWriter,
    logger: crate::logging::Logger,
}

impl JsonLinesPipeline {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            writer: FileWriter::default(),
            logger: get_logger("JsonLinesPipeline", None),
        }
    }
}

impl<S: Spider> ItemPipeline<S> for JsonLinesPipeline {
    fn open(&self, _spider: Arc<S>) -> PipelineFuture<'_, SilkwormResult<()>> {
        Box::pin(async move {
            let opening = self.writer.begin_open()?;
            if let Some(parent) = self.path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
                .await?;
            opening.start(
                file,
                RecordFormat::JsonLines,
                Vec::new(),
                self.logger.clone(),
            )?;
            self.logger.info(
                "Opened JSON Lines pipeline",
                &[("path", self.path.display().to_string())],
            );
            Ok(())
        })
    }

    fn close(&self, _spider: Arc<S>) -> PipelineFuture<'_, SilkwormResult<()>> {
        Box::pin(async move {
            self.writer.close().await?;
            self.logger.info(
                "Closed JSON Lines pipeline",
                &[("path", self.path.display().to_string())],
            );
            Ok(())
        })
    }

    fn process_item(
        &self,
        item: Item,
        _spider: Arc<S>,
    ) -> PipelineFuture<'_, SilkwormResult<Item>> {
        Box::pin(async move { self.writer.write(item, "JsonLinesPipeline").await })
    }
}

pub struct CsvPipeline {
    path: PathBuf,
    configured_fieldnames: Option<Vec<String>>,
    writer: FileWriter,
    logger: crate::logging::Logger,
}

impl CsvPipeline {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, fieldnames: Option<Vec<String>>) -> Self {
        Self {
            path: path.into(),
            configured_fieldnames: fieldnames,
            writer: FileWriter::default(),
            logger: get_logger("CsvPipeline", None),
        }
    }
}

impl<S: Spider> ItemPipeline<S> for CsvPipeline {
    fn open(&self, _spider: Arc<S>) -> PipelineFuture<'_, SilkwormResult<()>> {
        Box::pin(async move {
            let opening = self.writer.begin_open()?;
            if let Some(parent) = self.path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&self.path)
                .await?;
            opening.start(
                file,
                RecordFormat::Csv {
                    fieldnames: self.configured_fieldnames.clone(),
                    header_written: false,
                },
                Vec::new(),
                self.logger.clone(),
            )?;
            self.logger.info(
                "Opened CSV pipeline",
                &[("path", self.path.display().to_string())],
            );
            Ok(())
        })
    }

    fn close(&self, _spider: Arc<S>) -> PipelineFuture<'_, SilkwormResult<()>> {
        Box::pin(async move {
            self.writer.close().await?;
            self.logger.info(
                "Closed CSV pipeline",
                &[("path", self.path.display().to_string())],
            );
            Ok(())
        })
    }

    fn process_item(
        &self,
        item: Item,
        _spider: Arc<S>,
    ) -> PipelineFuture<'_, SilkwormResult<Item>> {
        Box::pin(async move { self.writer.write(item, "CsvPipeline").await })
    }
}

pub struct XmlPipeline {
    path: PathBuf,
    root_element: String,
    item_element: String,
    writer: FileWriter,
    logger: crate::logging::Logger,
}

impl XmlPipeline {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, root_element: &str, item_element: &str) -> Self {
        Self {
            path: path.into(),
            root_element: root_element.to_string(),
            item_element: item_element.to_string(),
            writer: FileWriter::default(),
            logger: get_logger("XmlPipeline", None),
        }
    }
}

impl<S: Spider> ItemPipeline<S> for XmlPipeline {
    fn open(&self, _spider: Arc<S>) -> PipelineFuture<'_, SilkwormResult<()>> {
        Box::pin(async move {
            let opening = self.writer.begin_open()?;
            if let Some(parent) = self.path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&self.path)
                .await?;
            let root = sanitize_tag(&self.root_element);
            let header = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<{root}>\n");
            file.write_all(header.as_bytes()).await?;
            opening.start(
                file,
                RecordFormat::Xml {
                    item_element: self.item_element.clone(),
                },
                format!("</{root}>\n").into_bytes(),
                self.logger.clone(),
            )?;
            self.logger.info(
                "Opened XML pipeline",
                &[("path", self.path.display().to_string())],
            );
            Ok(())
        })
    }

    fn close(&self, _spider: Arc<S>) -> PipelineFuture<'_, SilkwormResult<()>> {
        Box::pin(async move {
            self.writer.close().await?;
            self.logger.info(
                "Closed XML pipeline",
                &[("path", self.path.display().to_string())],
            );
            Ok(())
        })
    }

    fn process_item(
        &self,
        item: Item,
        _spider: Arc<S>,
    ) -> PipelineFuture<'_, SilkwormResult<Item>> {
        Box::pin(async move { self.writer.write(item, "XmlPipeline").await })
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
    use super::{
        CsvPipeline, ItemPipeline, JsonLinesPipeline, XmlPipeline, build_xml, flatten_item,
        sanitize_tag,
    };
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
        let mut output = Vec::new();
        super::csv::append_row(&mut output, ["plain", "a,b", "a\"b"].into_iter());
        assert_eq!(output, b"plain,\"a,b\",\"a\"\"b\"\n");
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
    async fn concurrent_file_pipeline_writes_preserve_every_item_and_document_structure() {
        let base = std::env::temp_dir().join(format!("silkworm_concurrent_{}", std::process::id()));
        let paths = [
            base.with_extension("jl"),
            base.with_extension("csv"),
            base.with_extension("xml"),
        ];
        let _ = tokio::fs::remove_file(&paths[0]).await;
        let pipelines: [Arc<dyn ItemPipeline<TestSpider>>; 3] = [
            Arc::new(JsonLinesPipeline::new(&paths[0])),
            Arc::new(CsvPipeline::new(&paths[1], None)),
            Arc::new(XmlPipeline::new(&paths[2], "items", "entry")),
        ];
        let spider = Arc::new(TestSpider);
        for (index, pipeline) in pipelines.into_iter().enumerate() {
            pipeline.open(spider.clone()).await.unwrap();
            // A second open must fail before it can truncate the live file.
            assert!(pipeline.open(spider.clone()).await.is_err());
            let mut tasks = tokio::task::JoinSet::new();
            for id in 0..32 {
                let pipeline = pipeline.clone();
                let spider = spider.clone();
                tasks.spawn(async move {
                    let item = serde_json::json!({"id": id});
                    assert_eq!(
                        pipeline.process_item(item.clone(), spider).await.unwrap(),
                        item
                    );
                });
            }
            while let Some(result) = tasks.join_next().await {
                result.unwrap();
            }
            pipeline.close(spider.clone()).await.unwrap();
            pipeline.close(spider.clone()).await.unwrap();
            let content = tokio::fs::read_to_string(&paths[index]).await.unwrap();
            match index {
                0 => {
                    let mut ids: Vec<u64> = content
                        .lines()
                        .map(|line| {
                            serde_json::from_str::<Item>(line).unwrap()["id"]
                                .as_u64()
                                .unwrap()
                        })
                        .collect();
                    ids.sort_unstable();
                    assert_eq!(ids, (0..32).collect::<Vec<_>>());
                }
                1 => {
                    let mut lines = content.lines();
                    assert_eq!(lines.next(), Some("id"));
                    let mut ids: Vec<u64> = lines.map(|line| line.parse().unwrap()).collect();
                    ids.sort_unstable();
                    assert_eq!(ids, (0..32).collect::<Vec<_>>());
                }
                _ => {
                    assert_eq!(content.matches("<entry>").count(), 32);
                    assert_eq!(content.matches("</entry>").count(), 32);
                    for id in 0..32 {
                        assert!(content.contains(&format!("<id>{id}</id>")));
                    }
                    assert!(content.ends_with("</items>\n"));
                    assert_eq!(content.matches("</items>").count(), 1);
                }
            }
            tokio::fs::remove_file(&paths[index]).await.unwrap();
        }
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
