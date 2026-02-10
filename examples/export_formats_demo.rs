use clap::Parser;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use silkworm::{crawl_with, prelude::*};

#[path = "support/mock_server.rs"]
mod mock_server;

const USER_AGENT: &str = "silkworm-rs/export-formats-demo";

struct ExportFormatsSpider {
    start_url: String,
    max_pages: usize,
    pages_scraped: AtomicUsize,
}

#[derive(Debug, Parser)]
#[command(
    name = "export_formats_demo",
    about = "Export quotes in multiple formats"
)]
struct Args {
    #[arg(long, value_name = "N", default_value_t = 2)]
    pages: usize,
    #[arg(long = "output-dir", value_name = "DIR", default_value = "data")]
    output_dir: PathBuf,
}

#[derive(Debug, Serialize)]
struct QuoteItem {
    text: String,
    author: String,
    tags: Vec<String>,
}

impl ExportFormatsSpider {
    fn new(start_url: String, max_pages: usize) -> Self {
        ExportFormatsSpider {
            start_url,
            max_pages: max_pages.max(1),
            pages_scraped: AtomicUsize::new(0),
        }
    }
}

impl Spider for ExportFormatsSpider {
    fn name(&self) -> &str {
        "export_formats"
    }

    async fn start_requests(&self) -> Vec<Request<Self>> {
        vec![Request::get(self.start_url.clone())]
    }

    async fn parse(&self, response: HtmlResponse<Self>) -> SpiderResult<Self> {
        let mut out = Vec::new();

        let page = self.pages_scraped.fetch_add(1, Ordering::SeqCst) + 1;
        for quote in response.select_or_empty(".quote") {
            let text = quote.text_from(".text");
            let author = quote.text_from(".author");
            if text.is_empty() || author.is_empty() {
                continue;
            }

            let quote = QuoteItem {
                text,
                author,
                tags: quote.select_texts(".tag"),
            };
            if let Ok(item) = item_from(quote) {
                out.push(item.into());
            }
        }

        if page < self.max_pages {
            let next_links = response
                .select_or_empty("li.next > a")
                .into_iter()
                .filter_map(|link| link.attr("href"))
                .collect::<Vec<_>>();
            out.extend(response.follow_urls(next_links).into_iter().map(Into::into));
        }

        Ok(out)
    }
}

#[tokio::main]
async fn main() -> silkworm::SilkwormResult<()> {
    let mock = mock_server::maybe_start().await?;
    let start_url = mock
        .as_ref()
        .map(|server| server.quotes_page_url(1))
        .unwrap_or_else(|| "https://quotes.toscrape.com/page/1/".to_string());
    let args = Args::parse();
    let max_pages = args.pages.max(1);
    let output_dir = args.output_dir;
    let json_path = output_dir.join("quotes_demo.jl");
    let xml_path = output_dir.join("quotes_demo.xml");
    let csv_path = output_dir.join("quotes_demo.csv");

    let config = RunConfig::new()
        .with_request_middleware(UserAgentMiddleware::new(
            vec![],
            Some(USER_AGENT.to_string()),
        ))
        .with_response_middleware(RetryMiddleware::new(3, None, None, 0.5))
        .with_item_pipeline(JsonLinesPipeline::new(json_path))
        .with_item_pipeline(XmlPipeline::new(xml_path, "quotes", "quote"))
        .with_item_pipeline(CsvPipeline::new(
            csv_path,
            Some(vec![
                "author".to_string(),
                "text".to_string(),
                "tags".to_string(),
            ]),
        ))
        .with_request_timeout(Duration::from_secs(10))
        .with_log_stats_interval(Duration::from_secs(10))
        .with_fail_fast(true);

    crawl_with(ExportFormatsSpider::new(start_url, max_pages), config).await
}
