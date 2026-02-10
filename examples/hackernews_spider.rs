use clap::Parser;
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use silkworm::{crawl_with, prelude::*};

const USER_AGENT: &str = "silkworm-rs/hackernews-spider";

struct HackerNewsSpider {
    max_pages: usize,
    pages_seen: AtomicUsize,
}

#[derive(Debug, Parser)]
#[command(
    name = "hackernews_spider",
    about = "Crawl the newest Hacker News pages"
)]
struct Args {
    #[arg(long, value_name = "N", default_value_t = 5)]
    pages: usize,
}

#[derive(Debug, Serialize)]
struct HackerNewsItem {
    title: String,
    url: String,
    author: Option<String>,
    points: Option<u64>,
    comments: Option<u64>,
    rank: Option<u64>,
    age: Option<String>,
    post_id: Option<u64>,
}

impl HackerNewsSpider {
    fn new(max_pages: usize) -> Self {
        HackerNewsSpider {
            max_pages: max_pages.max(1),
            pages_seen: AtomicUsize::new(0),
        }
    }
}

impl Spider for HackerNewsSpider {
    fn name(&self) -> &str {
        "hacker_news_latest"
    }

    fn start_urls(&self) -> Vec<&str> {
        vec!["https://news.ycombinator.com/newest"]
    }

    async fn parse(&self, response: HtmlResponse<Self>) -> SpiderResult<Self> {
        let mut out = Vec::new();

        let page = self.pages_seen.fetch_add(1, Ordering::SeqCst) + 1;
        for row in response.select_or_empty("tr.athing") {
            let post_id = row.attr("id");

            let rank = row
                .select_first_or_none(".rank")
                .and_then(|el| extract_number(&el.text()));

            let title_el = row.select_first_or_none("span.titleline a, a.storylink");
            let title = title_el.as_ref().map(|el| el.text()).unwrap_or_default();
            let href = title_el.and_then(|el| el.attr("href"));
            let url = href
                .as_deref()
                .map(|value| response.url_join(value))
                .unwrap_or_default();

            let subtext = post_id.as_ref().and_then(|id| {
                response.select_first_or_none(&format!("tr.athing[id='{id}'] + tr .subtext"))
            });

            let points = subtext
                .as_ref()
                .and_then(|el| el.select_first_or_none(".score"))
                .and_then(|el| extract_number(&el.text()));

            let comments = subtext.as_ref().and_then(|el| {
                el.select_or_empty("a").into_iter().find_map(|link| {
                    let text = link.text().to_lowercase();
                    if text.contains("comment") {
                        extract_number(&text)
                    } else {
                        None
                    }
                })
            });

            let author = subtext
                .as_ref()
                .and_then(|el| el.select_first_or_none("a.hnuser"))
                .map(|el| el.text());

            let age = subtext
                .as_ref()
                .and_then(|el| el.select_first_or_none(".age a"))
                .map(|el| el.text());

            let post_id = post_id.as_ref().and_then(|id| extract_number(id));
            let item = HackerNewsItem {
                title,
                url,
                author,
                points,
                comments,
                rank,
                age,
                post_id,
            };
            if let Ok(item) = item_from(item) {
                out.push(item.into());
            }
        }

        if page < self.max_pages {
            let next_links = response
                .select_or_empty("a.morelink")
                .into_iter()
                .filter_map(|link| link.attr("href"))
                .collect::<Vec<_>>();
            out.extend(response.follow_urls(next_links).into_iter().map(Into::into));
        }

        Ok(out)
    }
}

fn extract_number(text: &str) -> Option<u64> {
    let digits = text
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

#[tokio::main]
async fn main() -> silkworm::SilkwormResult<()> {
    let pages = Args::parse().pages.max(1);

    let request_middlewares: Vec<Arc<dyn RequestMiddleware<HackerNewsSpider>>> = vec![
        Arc::new(UserAgentMiddleware::new(
            vec![],
            Some(USER_AGENT.to_string()),
        )),
        Arc::new(DelayMiddleware::random(0.3, 1.0)),
    ];
    let response_middlewares: Vec<Arc<dyn ResponseMiddleware<HackerNewsSpider>>> = vec![Arc::new(
        RetryMiddleware::new(3, None, Some(vec![403]), 0.5),
    )];

    let config = RunConfig::new()
        .with_middlewares(request_middlewares, response_middlewares)
        .with_item_pipeline(JsonLinesPipeline::new("data/hackernews.jl"))
        .with_request_timeout(Duration::from_secs(10))
        .with_log_stats_interval(Duration::from_secs(10));

    crawl_with(HackerNewsSpider::new(pages), config).await
}
