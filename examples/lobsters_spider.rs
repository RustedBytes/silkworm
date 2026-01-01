use regex::Regex;
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use silkworm::{prelude::*, run_spider_with};

struct LobstersSpider {
    max_pages: usize,
    pages_seen: AtomicUsize,
}

#[derive(Debug, Serialize)]
struct LobstersItem {
    title: String,
    url: String,
    short_id: String,
    tags: Vec<String>,
    author: Option<String>,
    points: Option<u64>,
    comments: Option<u64>,
    age: Option<String>,
    domain: Option<String>,
}

impl LobstersSpider {
    fn new(max_pages: usize) -> Self {
        LobstersSpider {
            max_pages: max_pages.max(1),
            pages_seen: AtomicUsize::new(0),
        }
    }
}

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
        for story in response.select_or_empty("ol.stories > li.story") {
            let mut short_id = story
                .attr("data-shortid")
                .or_else(|| story.attr("id"))
                .unwrap_or_default();
            if let Some(value) = short_id.strip_prefix("story_") {
                short_id = value.to_string();
            }

            let title_el = story.select_first_or_none("span.link a.u-url");
            let title = title_el.as_ref().map(|el| el.text()).unwrap_or_default();
            let href = title_el.and_then(|el| el.attr("href"));
            let url = href
                .as_deref()
                .map(|value| response.url_join(value))
                .unwrap_or_default();

            let domain = story
                .select_first_or_none("a.domain")
                .map(|el| el.text());

            let tags = story
                .select_or_empty("span.tags a.tag")
                .into_iter()
                .map(|tag| tag.text())
                .collect::<Vec<_>>();

            let author = story
                .select_first_or_none(".byline .u-author")
                .map(|el| el.text());

            let age = story
                .select_first_or_none(".byline time")
                .map(|el| el.text());

            let comments = story
                .select_first_or_none(".comments_label a")
                .and_then(|el| extract_number(&el.text()));

            let points = story
                .select_first_or_none(".voters .upvoter")
                .and_then(|el| extract_number(&el.text()));

            let item = LobstersItem {
                title,
                url,
                short_id,
                tags,
                author,
                points,
                comments,
                age,
                domain,
            };
            if let Ok(item) = item_from(item) {
                out.push(item.into());
            }
        }

        if page < self.max_pages {
            let next_links = response
                .select_or_empty("div.morelink a[href]")
                .into_iter()
                .filter_map(|link| link.attr("href"))
                .collect::<Vec<_>>();
            out.extend(
                response
                    .follow_urls(next_links)
                    .into_iter()
                    .map(Into::into),
            );
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

    let request_middlewares: Vec<Arc<dyn RequestMiddleware<LobstersSpider>>> = vec![
        Arc::new(UserAgentMiddleware::new(vec![], None)),
        Arc::new(DelayMiddleware::random(0.3, 1.0)),
    ];
    let response_middlewares: Vec<Arc<dyn ResponseMiddleware<LobstersSpider>>> = vec![
        Arc::new(RetryMiddleware::new(
            15,
            None,
            Some(vec![403, 429]),
            0.5,
        )),
    ];

    let config = RunConfig::new()
        .with_concurrency(32)
        .with_middlewares(request_middlewares, response_middlewares)
        .with_item_pipeline(JsonLinesPipeline::new("data/lobsters.jl"))
        .with_request_timeout(Duration::from_secs(10))
        .with_log_stats_interval(Duration::from_secs(10));

    run_spider_with(LobstersSpider::new(pages), config)
}
