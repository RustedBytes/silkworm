use serde_json::{Map, Number, Value};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use silkworm::{
    HtmlResponse, JsonLinesPipeline, Request, RetryMiddleware, RunConfig, SkipNonHtmlMiddleware,
    Spider, SpiderResult, UserAgentMiddleware, run_spider_with,
};

struct UrlTitlesSpider {
    urls_path: PathBuf,
}

impl UrlTitlesSpider {
    fn new(urls_path: PathBuf) -> Self {
        UrlTitlesSpider { urls_path }
    }

    fn load_records(&self) -> Vec<Map<String, Value>> {
        let logger = self.log();
        let file = match File::open(&self.urls_path) {
            Ok(file) => file,
            Err(err) => {
                logger.error(
                    "Failed to open URLs file",
                    &[
                        ("error", err.to_string()),
                        ("path", self.urls_path.display().to_string()),
                    ],
                );
                return Vec::new();
            }
        };

        let reader = BufReader::new(file);
        let mut records = Vec::new();
        for (line_no, line) in reader.lines().enumerate() {
            let line = match line {
                Ok(value) => value,
                Err(err) => {
                    logger.warn(
                        "Skipping unreadable line",
                        &[
                            ("line", (line_no + 1).to_string()),
                            ("error", err.to_string()),
                        ],
                    );
                    continue;
                }
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let value: Value = match serde_json::from_str(trimmed) {
                Ok(value) => value,
                Err(err) => {
                    logger.warn(
                        "Skipping invalid JSON line",
                        &[
                            ("line", (line_no + 1).to_string()),
                            ("error", err.to_string()),
                        ],
                    );
                    continue;
                }
            };

            let mut map = match value {
                Value::Object(map) => map,
                _ => {
                    logger.warn(
                        "Skipping non-object JSON line",
                        &[("line", (line_no + 1).to_string())],
                    );
                    continue;
                }
            };

            let url = map
                .get("url")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if url.is_empty() {
                logger.warn(
                    "Skipping line without url field",
                    &[("line", (line_no + 1).to_string())],
                );
                continue;
            }
            map.insert("url".to_string(), Value::String(url));
            records.push(map);
        }

        logger.info(
            "Loaded URLs",
            &[
                ("count", records.len().to_string()),
                ("path", self.urls_path.display().to_string()),
            ],
        );
        records
    }
}

impl Spider for UrlTitlesSpider {
    fn name(&self) -> &str {
        "url_titles_from_file"
    }

    async fn start_requests(&self) -> Vec<Request<Self>> {
        let records = self.load_records();
        let mut out = Vec::new();

        for record in records {
            let url = record
                .get("url")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string();
            if url.is_empty() {
                continue;
            }
            let mut req = Request::new(url);
            req.meta.insert("record".to_string(), Value::Object(record));
            req.dont_filter = true;
            out.push(req);
        }

        out
    }

    async fn parse(&self, response: HtmlResponse<Self>) -> SpiderResult<Self> {
        let mut record = match response.request.meta.get("record") {
            Some(Value::Object(map)) => map.clone(),
            _ => Map::new(),
        };

        let title = response
            .select_first("title")
            .ok()
            .flatten()
            .map(|el| el.text().trim().to_string())
            .unwrap_or_default();

        record.insert("page_title".to_string(), Value::String(title));
        record.insert("final_url".to_string(), Value::String(response.url.clone()));
        record.insert(
            "status".to_string(),
            Value::Number(Number::from(u64::from(response.status))),
        );

        vec![Value::Object(record).into()]
    }
}

fn parse_args() -> Result<(PathBuf, PathBuf), String> {
    let mut urls_file: Option<PathBuf> = None;
    let mut output = PathBuf::from("data/url_titles.jl");

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--urls-file" => {
                if let Some(value) = args.next() {
                    urls_file = Some(PathBuf::from(value));
                }
            }
            "--output" => {
                if let Some(value) = args.next() {
                    output = PathBuf::from(value);
                }
            }
            _ => {}
        }
    }

    let Some(urls_file) = urls_file else {
        return Err("Missing --urls-file argument".to_string());
    };

    Ok((urls_file, output))
}

fn main() -> silkworm::SilkwormResult<()> {
    let (urls_file, output) = match parse_args() {
        Ok(values) => values,
        Err(err) => {
            eprintln!("{err}");
            eprintln!(
                "Usage: cargo run --example url_titles_spider -- --urls-file <path> [--output <path>]"
            );
            std::process::exit(1);
        }
    };

    let mut config = RunConfig::default();
    config.concurrency = 128;
    config.request_middlewares = vec![Arc::new(UserAgentMiddleware::new(vec![], None))];
    config.response_middlewares = vec![
        Arc::new(RetryMiddleware::new(3, None, Some(vec![403, 429]), 0.5)),
        Arc::new(SkipNonHtmlMiddleware::new(None, 1024)),
    ];
    config.item_pipelines = vec![Arc::new(JsonLinesPipeline::new(output))];
    config.request_timeout = Some(Duration::from_secs(5));
    config.log_stats_interval = Some(Duration::from_secs(10));
    config.html_max_size_bytes = 1_000_000;
    config.keep_alive = true;

    run_spider_with(UrlTitlesSpider::new(urls_file), config)
}
