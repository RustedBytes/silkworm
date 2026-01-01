use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use silkworm::{
    CsvPipeline, HtmlResponse, JsonLinesPipeline, RetryMiddleware, RunConfig, Spider, SpiderResult,
    UserAgentMiddleware, XmlPipeline, run_spider_with,
};

struct ExportFormatsSpider {
    max_pages: usize,
    pages_scraped: AtomicUsize,
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

    fn start_urls(&self) -> Vec<String> {
        vec!["https://quotes.toscrape.com/page/1/".to_string()]
    }

    async fn parse(&self, response: HtmlResponse<Self>) -> SpiderResult<Self> {
        let mut out = Vec::new();

        let page = self.pages_scraped.fetch_add(1, Ordering::SeqCst) + 1;
        let quotes = match response.select(".quote") {
            Ok(nodes) => nodes,
            Err(_) => return out,
        };

        for quote in quotes {
            let text_el = match quote.select_first(".text") {
                Ok(Some(el)) => el,
                _ => continue,
            };
            let author_el = match quote.select_first(".author") {
                Ok(Some(el)) => el,
                _ => continue,
            };
            let tags = match quote.select(".tag") {
                Ok(nodes) => nodes,
                Err(_) => Vec::new(),
            };
            let tag_values = tags.into_iter().map(|tag| tag.text()).collect::<Vec<_>>();

            out.push(
                json!({
                    "text": text_el.text(),
                    "author": author_el.text(),
                    "tags": tag_values,
                })
                .into(),
            );
        }

        if page < self.max_pages {
            if let Ok(Some(link)) = response.select_first("li.next > a") {
                if let Some(href) = link.attr("href") {
                    out.push(response.follow(&href, None).into());
                }
            }
        }

        out
    }
}

fn parse_pages_arg() -> usize {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--pages" {
            if let Some(value) = args.next() {
                if let Ok(parsed) = value.parse::<usize>() {
                    return parsed.max(1);
                }
            }
        } else if let Some(value) = arg.strip_prefix("--pages=") {
            if let Ok(parsed) = value.parse::<usize>() {
                return parsed.max(1);
            }
        }
    }
    2
}

fn main() -> silkworm::SilkwormResult<()> {
    let max_pages = parse_pages_arg();

    let mut config = RunConfig::default();
    config.request_middlewares = vec![Arc::new(UserAgentMiddleware::new(vec![], None))];
    config.response_middlewares = vec![Arc::new(RetryMiddleware::new(3, None, None, 0.5))];
    config.item_pipelines = vec![
        Arc::new(JsonLinesPipeline::new("data/quotes_demo.jl")),
        Arc::new(XmlPipeline::new("data/quotes_demo.xml", "quotes", "quote")),
        Arc::new(CsvPipeline::new(
            "data/quotes_demo.csv",
            Some(vec![
                "author".to_string(),
                "text".to_string(),
                "tags".to_string(),
            ]),
        )),
    ];
    config.request_timeout = Some(Duration::from_secs(10));

    run_spider_with(ExportFormatsSpider::new(max_pages), config)
}
