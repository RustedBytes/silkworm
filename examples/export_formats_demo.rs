use clap::Parser;
use serde::Serialize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use silkworm::{crawl_with, prelude::*};

struct ExportFormatsSpider {
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
}

#[derive(Debug, Serialize)]
struct QuoteItem {
    text: String,
    author: String,
    tags: Vec<String>,
}

impl ExportFormatsSpider {
    fn new(max_pages: usize) -> Self {
        ExportFormatsSpider {
            max_pages: max_pages.max(1),
            pages_scraped: AtomicUsize::new(0),
        }
    }
}

impl Spider for ExportFormatsSpider {
    fn name(&self) -> &str {
        "export_formats"
    }

    fn start_urls(&self) -> Vec<&str> {
        vec!["https://quotes.toscrape.com/page/1/"]
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
            let tag_values = quote
                .select_or_empty(".tag")
                .into_iter()
                .map(|tag| tag.text())
                .collect::<Vec<_>>();

            let quote = QuoteItem {
                text,
                author,
                tags: tag_values,
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

        out
    }
}

#[tokio::main]
async fn main() -> silkworm::SilkwormResult<()> {
    let max_pages = Args::parse().pages.max(1);

    let config = RunConfig::new()
        .with_request_middleware(UserAgentMiddleware::new(vec![], None))
        .with_response_middleware(RetryMiddleware::new(3, None, None, 0.5))
        .with_item_pipeline(JsonLinesPipeline::new("data/quotes_demo.jl"))
        .with_item_pipeline(XmlPipeline::new("data/quotes_demo.xml", "quotes", "quote"))
        .with_item_pipeline(CsvPipeline::new(
            "data/quotes_demo.csv",
            Some(vec![
                "author".to_string(),
                "text".to_string(),
                "tags".to_string(),
            ]),
        ))
        .with_request_timeout(Duration::from_secs(10));

    crawl_with(ExportFormatsSpider::new(max_pages), config).await
}
