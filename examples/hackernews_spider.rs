use async_trait::async_trait;
use serde_json::{Number, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use silkworm::{
    run_spider_with, DelayMiddleware, HtmlResponse, JsonLinesPipeline, RetryMiddleware, RunConfig,
    Spider, SpiderResult, UserAgentMiddleware,
};

struct HackerNewsSpider {
    max_pages: usize,
    pages_seen: AtomicUsize,
}

impl HackerNewsSpider {
    fn new(max_pages: usize) -> Self {
        HackerNewsSpider {
            max_pages: max_pages.max(1),
            pages_seen: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl Spider for HackerNewsSpider {
    fn name(&self) -> &str {
        "hacker_news_latest"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://news.ycombinator.com/newest".to_string()]
    }

    async fn parse(&self, response: HtmlResponse<Self>) -> SpiderResult<Self> {
        let mut out = Vec::new();

        let page = self.pages_seen.fetch_add(1, Ordering::SeqCst) + 1;
        let rows = match response.select("tr.athing") {
            Ok(nodes) => nodes,
            Err(_) => return out,
        };

        for row in rows {
            let post_id = row.attr("id");

            let rank = row
                .select_first(".rank")
                .ok()
                .flatten()
                .and_then(|el| extract_number(&el.text()));

            let title_el = row
                .select_first("span.titleline a, a.storylink")
                .ok()
                .flatten();
            let title = title_el.as_ref().map(|el| el.text()).unwrap_or_default();
            let href = title_el.and_then(|el| el.attr("href"));
            let url = href
                .as_deref()
                .map(|value| response.url_join(value))
                .unwrap_or_default();

            let subtext = post_id.as_ref().and_then(|id| {
                response
                    .select_first(&format!("tr.athing[id='{id}'] + tr .subtext"))
                    .ok()
                    .flatten()
            });

            let points = subtext
                .as_ref()
                .and_then(|el| el.select_first(".score").ok().flatten())
                .and_then(|el| extract_number(&el.text()));

            let comments = subtext.as_ref().and_then(|el| {
                el.select("a")
                    .ok()
                    .unwrap_or_default()
                    .into_iter()
                    .find_map(|link| {
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
                .and_then(|el| el.select_first("a.hnuser").ok().flatten())
                .map(|el| el.text());

            let age = subtext
                .as_ref()
                .and_then(|el| el.select_first(".age a").ok().flatten())
                .map(|el| el.text());

            let mut item = serde_json::Map::new();
            item.insert("title".to_string(), Value::String(title));
            item.insert("url".to_string(), Value::String(url));

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
            if let Some(rank) = rank {
                item.insert("rank".to_string(), Value::Number(Number::from(rank)));
            }
            if let Some(age) = age {
                item.insert("age".to_string(), Value::String(age));
            }
            if let Some(post_id) = post_id {
                if let Some(id_value) = extract_number(&post_id) {
                    item.insert("post_id".to_string(), Value::Number(Number::from(id_value)));
                }
            }

            out.push(Value::Object(item).into());
        }

        if page < self.max_pages {
            if let Ok(Some(link)) = response.select_first("a.morelink") {
                if let Some(href) = link.attr("href") {
                    out.push(response.follow(&href, None).into());
                }
            }
        }

        out
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
    5
}

fn main() -> silkworm::SilkwormResult<()> {
    let pages = parse_pages_arg();

    let mut config = RunConfig::default();
    config.request_middlewares = vec![
        Arc::new(UserAgentMiddleware::new(vec![], None)),
        Arc::new(DelayMiddleware::random(0.3, 1.0)),
    ];
    config.response_middlewares = vec![Arc::new(RetryMiddleware::new(
        3,
        None,
        Some(vec![403]),
        0.5,
    ))];
    config.item_pipelines = vec![Arc::new(JsonLinesPipeline::new("data/hackernews.jl"))];
    config.request_timeout = Some(Duration::from_secs(10));
    config.log_stats_interval = Some(Duration::from_secs(10));

    run_spider_with(HackerNewsSpider::new(pages), config)
}
