use async_trait::async_trait;
use regex::Regex;
use serde_json::{Number, Value};
use std::future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use silkworm::{
    callback_from_fn, run_spider_with, CallbackFuture, DelayMiddleware, HtmlResponse,
    JsonLinesPipeline, Request, Response, RetryMiddleware, RunConfig, SkipNonHtmlMiddleware,
    Spider, SpiderResult, UserAgentMiddleware,
};

struct SitemapSpider {
    sitemap_url: String,
    max_pages: Option<usize>,
    pages_seen: AtomicUsize,
}

impl SitemapSpider {
    fn new(sitemap_url: String, max_pages: Option<usize>) -> Self {
        SitemapSpider {
            sitemap_url,
            max_pages,
            pages_seen: AtomicUsize::new(0),
        }
    }

    async fn parse_page(&self, response: HtmlResponse<Self>) -> SpiderResult<Self> {
        let mut item = serde_json::Map::new();
        item.insert("url".to_string(), Value::String(response.url.clone()));
        item.insert(
            "status".to_string(),
            Value::Number(Number::from(u64::from(response.status))),
        );

        if let Ok(Some(title_el)) = response.select_first("title") {
            let title = title_el.text().trim().to_string();
            if !title.is_empty() {
                item.insert("title".to_string(), Value::String(title));
            }
        }

        if let Ok(Some(canonical_el)) = response.select_first("link[rel='canonical']") {
            if let Some(href) = canonical_el.attr("href") {
                item.insert("canonical_url".to_string(), Value::String(href));
            }
        }

        if let Ok(meta_tags) = response.select("meta") {
            for meta in meta_tags {
                let name = meta.attr("name").or_else(|| meta.attr("property"));
                let content = meta.attr("content");
                let Some(name) = name else { continue };
                let Some(content) = content else { continue };
                let key = match name.to_lowercase().as_str() {
                    "description" => "meta_description",
                    "keywords" => "meta_keywords",
                    "author" => "author",
                    "robots" => "robots",
                    "viewport" => "viewport",
                    "og:title" => "og_title",
                    "og:description" => "og_description",
                    "og:type" => "og_type",
                    "og:url" => "og_url",
                    "og:image" => "og_image",
                    "og:site_name" => "og_site_name",
                    "og:locale" => "og_locale",
                    "twitter:card" => "twitter_card",
                    "twitter:title" => "twitter_title",
                    "twitter:description" => "twitter_description",
                    "twitter:image" => "twitter_image",
                    "twitter:site" => "twitter_site",
                    _ => continue,
                };
                item.insert(key.to_string(), Value::String(content.trim().to_string()));
            }
        }

        vec![Value::Object(item).into()]
    }
}

#[async_trait]
impl Spider for SitemapSpider {
    fn name(&self) -> &str {
        "sitemap_metadata"
    }

    async fn start_requests(&self) -> Vec<Request<Self>> {
        vec![Request::new(self.sitemap_url.clone())
            .with_callback(callback_from_fn(parse_sitemap))
            .with_meta("allow_non_html", Value::Bool(true))
            .with_dont_filter(true)]
    }

    async fn parse(&self, response: HtmlResponse<Self>) -> SpiderResult<Self> {
        self.parse_page(response).await
    }
}

fn parse_sitemap(
    spider: Arc<SitemapSpider>,
    response: Response<SitemapSpider>,
) -> CallbackFuture<SitemapSpider> {
    Box::pin(future::ready(parse_sitemap_inner(spider, response)))
}

fn parse_sitemap_inner(
    spider: Arc<SitemapSpider>,
    response: Response<SitemapSpider>,
) -> SpiderResult<SitemapSpider> {
    let logger = spider.log();
    if response.status >= 400 {
        logger.warn(
            "Failed to fetch sitemap",
            &[
                ("url", response.url.clone()),
                ("status", response.status.to_string()),
            ],
        );
        return Vec::new();
    }

    let body = response.text();
    let is_index = body.to_lowercase().contains("<sitemapindex");
    let urls = extract_sitemap_urls(&body);

    if urls.is_empty() {
        logger.warn("No URLs found in sitemap", &[("url", response.url.clone())]);
        return Vec::new();
    }

    let mut out = Vec::new();
    for url in urls {
        if is_index {
            let req = Request::new(url)
                .with_callback(callback_from_fn(parse_sitemap))
                .with_meta("allow_non_html", Value::Bool(true))
                .with_dont_filter(true);
            out.push(req.into());
            continue;
        }

        if let Some(max_pages) = spider.max_pages {
            let next = spider.pages_seen.fetch_add(1, Ordering::SeqCst) + 1;
            if next > max_pages {
                break;
            }
        }

        let req = Request::new(url).with_dont_filter(true);
        out.push(req.into());
    }

    out
}

fn extract_sitemap_urls(body: &str) -> Vec<String> {
    let re = Regex::new(r"(?is)<loc>\s*([^<]+?)\s*</loc>").ok();
    let Some(re) = re else { return Vec::new() };
    re.captures_iter(body)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str().trim().to_string()))
        .collect()
}

fn parse_args() -> Result<(String, Option<usize>, String, usize, f64), String> {
    let mut sitemap_url: Option<String> = None;
    let mut output = "data/sitemap_meta.jl".to_string();
    let mut pages: Option<usize> = None;
    let mut concurrency = 16usize;
    let mut delay = 0f64;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sitemap-url" => {
                sitemap_url = args.next();
            }
            "--output" => {
                if let Some(value) = args.next() {
                    output = value;
                }
            }
            "--pages" => {
                if let Some(value) = args.next() {
                    pages = value.parse::<usize>().ok();
                }
            }
            "--concurrency" => {
                if let Some(value) = args.next() {
                    concurrency = value.parse::<usize>().unwrap_or(concurrency);
                }
            }
            "--delay" => {
                if let Some(value) = args.next() {
                    delay = value.parse::<f64>().unwrap_or(delay);
                }
            }
            _ => {}
        }
    }

    let Some(sitemap_url) = sitemap_url else {
        return Err("Missing --sitemap-url argument".to_string());
    };

    Ok((sitemap_url, pages, output, concurrency, delay))
}

fn main() -> silkworm::SilkwormResult<()> {
    let (sitemap_url, pages, output, concurrency, delay) = match parse_args() {
        Ok(values) => values,
        Err(err) => {
            eprintln!("{err}");
            eprintln!(
                "Usage: cargo run --example sitemap_spider -- --sitemap-url <url> [--output <path>] [--pages <n>] [--concurrency <n>] [--delay <seconds>]"
            );
            std::process::exit(1);
        }
    };

    let mut config = RunConfig::default();
    config.concurrency = concurrency;
    config.request_middlewares = vec![Arc::new(UserAgentMiddleware::new(vec![], None))];
    if delay > 0.0 {
        config
            .request_middlewares
            .push(Arc::new(DelayMiddleware::fixed(delay)));
    }
    config.response_middlewares = vec![
        Arc::new(RetryMiddleware::new(
            3,
            None,
            Some(vec![403, 429, 503]),
            0.5,
        )),
        Arc::new(SkipNonHtmlMiddleware::new(None, 1024)),
    ];
    config.item_pipelines = vec![Arc::new(JsonLinesPipeline::new(output))];
    config.request_timeout = Some(Duration::from_secs(30));
    config.log_stats_interval = Some(Duration::from_secs(10));
    config.html_max_size_bytes = 2_000_000;

    run_spider_with(SitemapSpider::new(sitemap_url, pages), config)
}
