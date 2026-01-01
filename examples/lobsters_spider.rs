use async_trait::async_trait;
use regex::Regex;
use serde_json::{Number, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use silkworm::{
    run_spider_with, DelayMiddleware, HtmlResponse, JsonLinesPipeline, RetryMiddleware, RunConfig,
    Spider, SpiderResult, UserAgentMiddleware,
};

struct LobstersSpider {
    max_pages: usize,
    pages_seen: AtomicUsize,
}

impl LobstersSpider {
    fn new(max_pages: usize) -> Self {
        LobstersSpider {
            max_pages: max_pages.max(1),
            pages_seen: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl Spider for LobstersSpider {
    fn name(&self) -> &str {
        "lobsters_front_page"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://lobste.rs/".to_string()]
    }

    async fn parse(&self, response: HtmlResponse<Self>) -> SpiderResult<Self> {
        let mut out = Vec::new();

        let page = self.pages_seen.fetch_add(1, Ordering::SeqCst) + 1;
        let stories = match response.select("ol.stories > li.story") {
            Ok(nodes) => nodes,
            Err(_) => return out,
        };

        for story in stories {
            let mut short_id = story
                .attr("data-shortid")
                .or_else(|| story.attr("id"))
                .unwrap_or_default();
            if let Some(value) = short_id.strip_prefix("story_") {
                short_id = value.to_string();
            }

            let title_el = story.select_first("span.link a.u-url").ok().flatten();
            let title = title_el.as_ref().map(|el| el.text()).unwrap_or_default();
            let href = title_el.and_then(|el| el.attr("href"));
            let url = href
                .as_deref()
                .map(|value| response.url_join(value))
                .unwrap_or_default();

            let domain = story
                .select_first("a.domain")
                .ok()
                .flatten()
                .map(|el| el.text());

            let tags = story
                .select("span.tags a.tag")
                .ok()
                .unwrap_or_default()
                .into_iter()
                .map(|tag| tag.text())
                .collect::<Vec<_>>();

            let author = story
                .select_first(".byline .u-author")
                .ok()
                .flatten()
                .map(|el| el.text());

            let age = story
                .select_first(".byline time")
                .ok()
                .flatten()
                .map(|el| el.text());

            let comments = story
                .select_first(".comments_label a")
                .ok()
                .flatten()
                .and_then(|el| extract_number(&el.text()));

            let points = story
                .select_first(".voters .upvoter")
                .ok()
                .flatten()
                .and_then(|el| extract_number(&el.text()));

            let mut item = serde_json::Map::new();
            item.insert("title".to_string(), Value::String(title));
            item.insert("url".to_string(), Value::String(url));
            item.insert("short_id".to_string(), Value::String(short_id));
            item.insert(
                "tags".to_string(),
                Value::Array(tags.into_iter().map(Value::String).collect()),
            );

            if let Some(author) = author {
                item.insert("author".to_string(), Value::String(author));
            }
            if let Some(points) = points {
                item.insert("points".to_string(), Value::Number(Number::from(points)));
            }
            if let Some(comments) = comments {
                item.insert(
                    "comments".to_string(),
                    Value::Number(Number::from(comments)),
                );
            }
            if let Some(age) = age {
                item.insert("age".to_string(), Value::String(age));
            }
            if let Some(domain) = domain {
                item.insert("domain".to_string(), Value::String(domain));
            }

            out.push(Value::Object(item).into());
        }

        if page < self.max_pages {
            let next_links = response
                .select("div.morelink a[href]")
                .ok()
                .unwrap_or_default();
            if let Some(link) = next_links.last() {
                if let Some(href) = link.attr("href") {
                    out.push(response.follow(&href, None).into());
                }
            }
        }

        out
    }
}

fn extract_number(text: &str) -> Option<u64> {
    let re = Regex::new(r"\d+").ok()?;
    re.find(text.replace(',', "").as_str())
        .and_then(|mat| mat.as_str().parse().ok())
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
    1
}

fn main() -> silkworm::SilkwormResult<()> {
    let pages = parse_pages_arg();

    let mut config = RunConfig::default();
    config.concurrency = 32;
    config.request_middlewares = vec![
        Arc::new(UserAgentMiddleware::new(vec![], None)),
        Arc::new(DelayMiddleware::random(0.3, 1.0)),
    ];
    config.response_middlewares = vec![Arc::new(RetryMiddleware::new(
        15,
        None,
        Some(vec![403, 429]),
        0.5,
    ))];
    config.item_pipelines = vec![Arc::new(JsonLinesPipeline::new("data/lobsters.jl"))];
    config.request_timeout = Some(Duration::from_secs(10));
    config.log_stats_interval = Some(Duration::from_secs(10));

    run_spider_with(LobstersSpider::new(pages), config)
}
