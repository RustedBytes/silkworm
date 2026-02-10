use clap::Parser;
use regex::Regex;
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use silkworm::{Response, crawl_with, prelude::*};

struct SitemapSpider {
    sitemap_url: String,
    max_pages: Option<usize>,
    pages_seen: AtomicUsize,
}

#[derive(Debug, Parser)]
#[command(
    name = "sitemap_spider",
    about = "Crawl a sitemap and collect metadata"
)]
struct Args {
    #[arg(long = "sitemap-url", value_name = "URL")]
    sitemap_url: String,
    #[arg(long, value_name = "PATH", default_value = "data/sitemap_meta.jl")]
    output: String,
    #[arg(long, value_name = "N")]
    pages: Option<usize>,
    #[arg(long, value_name = "N", default_value_t = 16)]
    concurrency: usize,
    #[arg(long, value_name = "SECONDS", default_value_t = 0.0)]
    delay: f64,
}

#[derive(Debug, Serialize)]
struct SitemapItem {
    url: String,
    status: u16,
    title: Option<String>,
    canonical_url: Option<String>,
    meta_description: Option<String>,
    meta_keywords: Option<String>,
    author: Option<String>,
    robots: Option<String>,
    viewport: Option<String>,
    og_title: Option<String>,
    og_description: Option<String>,
    og_type: Option<String>,
    og_url: Option<String>,
    og_image: Option<String>,
    og_site_name: Option<String>,
    og_locale: Option<String>,
    twitter_card: Option<String>,
    twitter_title: Option<String>,
    twitter_description: Option<String>,
    twitter_image: Option<String>,
    twitter_site: Option<String>,
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
        let mut item = SitemapItem {
            url: response.url.clone(),
            status: response.status,
            title: None,
            canonical_url: None,
            meta_description: None,
            meta_keywords: None,
            author: None,
            robots: None,
            viewport: None,
            og_title: None,
            og_description: None,
            og_type: None,
            og_url: None,
            og_image: None,
            og_site_name: None,
            og_locale: None,
            twitter_card: None,
            twitter_title: None,
            twitter_description: None,
            twitter_image: None,
            twitter_site: None,
        };

        let title = response.text_from("title");
        let title = title.trim();
        if !title.is_empty() {
            item.title = Some(title.to_string());
        }

        if let Some(href) = response.attr_from("link[rel='canonical']", "href") {
            item.canonical_url = Some(href);
        }

        for meta in response.select_or_empty("meta") {
            let name = meta.attr("name").or_else(|| meta.attr("property"));
            let content = meta.attr("content");
            let Some(name) = name else { continue };
            let Some(content) = content else { continue };
            let value = content.trim().to_string();
            match name.to_lowercase().as_str() {
                "description" => item.meta_description = Some(value),
                "keywords" => item.meta_keywords = Some(value),
                "author" => item.author = Some(value),
                "robots" => item.robots = Some(value),
                "viewport" => item.viewport = Some(value),
                "og:title" => item.og_title = Some(value),
                "og:description" => item.og_description = Some(value),
                "og:type" => item.og_type = Some(value),
                "og:url" => item.og_url = Some(value),
                "og:image" => item.og_image = Some(value),
                "og:site_name" => item.og_site_name = Some(value),
                "og:locale" => item.og_locale = Some(value),
                "twitter:card" => item.twitter_card = Some(value),
                "twitter:title" => item.twitter_title = Some(value),
                "twitter:description" => item.twitter_description = Some(value),
                "twitter:image" => item.twitter_image = Some(value),
                "twitter:site" => item.twitter_site = Some(value),
                _ => continue,
            }
        }

        if let Ok(item) = item_from(item) {
            Ok(vec![item.into()])
        } else {
            Ok(Vec::new())
        }
    }
}

impl Spider for SitemapSpider {
    fn name(&self) -> &str {
        "sitemap_metadata"
    }

    async fn start_requests(&self) -> Vec<Request<Self>> {
        vec![
            Request::get(self.sitemap_url.clone())
                .with_callback_fn(parse_sitemap)
                .with_allow_non_html(true)
                .with_dont_filter(true),
        ]
    }

    async fn parse(&self, response: HtmlResponse<Self>) -> SpiderResult<Self> {
        self.parse_page(response).await
    }
}

async fn parse_sitemap(
    spider: Arc<SitemapSpider>,
    response: Response<SitemapSpider>,
) -> SpiderResult<SitemapSpider> {
    parse_sitemap_inner(spider, response)
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
        return Ok(Vec::new());
    }

    let body = response.text();
    let is_index = body.to_lowercase().contains("<sitemapindex");
    let urls = extract_sitemap_urls(&body);

    if urls.is_empty() {
        logger.warn("No URLs found in sitemap", &[("url", response.url.clone())]);
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for url in urls {
        if is_index {
            let req = Request::get(url)
                .with_callback_fn(parse_sitemap)
                .with_allow_non_html(true)
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

        let req = Request::builder(url).dont_filter(true).build();
        out.push(req.into());
    }

    Ok(out)
}

fn extract_sitemap_urls(body: &str) -> Vec<String> {
    let re = Regex::new(r"(?is)<loc>\s*([^<]+?)\s*</loc>").ok();
    let Some(re) = re else { return Vec::new() };
    re.captures_iter(body)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str().trim().to_string()))
        .collect()
}

#[tokio::main]
async fn main() -> silkworm::SilkwormResult<()> {
    let args = Args::parse();
    let sitemap_url = args.sitemap_url;
    let pages = args.pages;
    let output = args.output;
    let concurrency = args.concurrency;
    let delay = args.delay;

    let mut request_middlewares: Vec<Arc<dyn RequestMiddleware<SitemapSpider>>> =
        vec![Arc::new(UserAgentMiddleware::new(vec![], None))];
    if delay > 0.0 {
        request_middlewares.push(Arc::new(DelayMiddleware::fixed(delay)));
    }
    let response_middlewares: Vec<Arc<dyn ResponseMiddleware<SitemapSpider>>> = vec![
        Arc::new(RetryMiddleware::new(
            3,
            None,
            Some(vec![403, 429, 503]),
            0.5,
        )),
        Arc::new(SkipNonHtmlMiddleware::new(None, 1024)),
    ];

    let config = RunConfig::new()
        .with_concurrency(concurrency)
        .with_middlewares(request_middlewares, response_middlewares)
        .with_item_pipeline(JsonLinesPipeline::new(output))
        .with_request_timeout(Duration::from_secs(30))
        .with_log_stats_interval(Duration::from_secs(10))
        .with_html_max_size_bytes(2_000_000);

    crawl_with(SitemapSpider::new(sitemap_url, pages), config).await
}
