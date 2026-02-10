use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::fmt::Write as _;
use std::hint::black_box;
use std::time::{Duration, Instant};

use bytes::Bytes;
use silkworm::{Headers, Request, Response};
use sxd_xpath::Factory;

const WARMUP_TIME: Duration = Duration::from_millis(200);
const SAMPLE_TIME: Duration = Duration::from_millis(800);
const HTML_MAX_SIZE_BYTES: usize = 2_000_000;
const SELECT_CACHED_RATIO_MAX: f64 = 0.70;
const XPATH_PRECOMPILED_RATIO_MAX: f64 = 1.20;
const SCHEDULER_PRIORITY_HEAP_NS_MAX: f64 = 3_000_000.0;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct SchedulerEntry {
    priority: i32,
    sequence: u64,
}

impl Ord for SchedulerEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.priority.cmp(&other.priority) {
            Ordering::Equal => other.sequence.cmp(&self.sequence),
            order => order,
        }
    }
}

impl PartialOrd for SchedulerEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

struct BenchResult {
    name: &'static str,
    iterations: u64,
    elapsed: Duration,
    ns_per_iter: f64,
    iter_per_sec: f64,
}

impl BenchResult {
    fn new(name: &'static str, iterations: u64, elapsed: Duration) -> Self {
        let iterations = iterations.max(1);
        let seconds = elapsed.as_secs_f64().max(f64::EPSILON);
        let ns_per_iter = elapsed.as_nanos() as f64 / iterations as f64;
        let iter_per_sec = iterations as f64 / seconds;
        BenchResult {
            name,
            iterations,
            elapsed,
            ns_per_iter,
            iter_per_sec,
        }
    }
}

fn run_bench<R, F>(name: &'static str, mut f: F) -> BenchResult
where
    F: FnMut() -> R,
{
    let warmup_start = Instant::now();
    while warmup_start.elapsed() < WARMUP_TIME {
        black_box(f());
    }

    let start = Instant::now();
    let mut iterations = 0u64;
    loop {
        black_box(f());
        iterations += 1;
        if start.elapsed() >= SAMPLE_TIME {
            break;
        }
    }

    BenchResult::new(name, iterations, start.elapsed())
}

fn render_results(results: &[BenchResult]) {
    println!("Silkworm benchmark suite (custom harness)");
    println!(
        "{:<36} {:>12} {:>12} {:>12}",
        "benchmark", "iters", "ns/iter", "iter/s"
    );
    for result in results {
        println!(
            "{:<36} {:>12} {:>12.2} {:>12.2}",
            result.name, result.iterations, result.ns_per_iter, result.iter_per_sec
        );
    }
    let total_elapsed: Duration = results.iter().map(|result| result.elapsed).sum();
    println!("total measured time: {:.2}s", total_elapsed.as_secs_f64());
}

fn benchmark_check_enabled() -> bool {
    std::env::var("SILKWORM_BENCH_CHECK")
        .map(|value| {
            let value = value.trim();
            value == "1"
                || value.eq_ignore_ascii_case("true")
                || value.eq_ignore_ascii_case("yes")
                || value.eq_ignore_ascii_case("on")
        })
        .unwrap_or(false)
}

fn benchmark_result<'a>(results: &'a [BenchResult], name: &str) -> Option<&'a BenchResult> {
    results.iter().find(|result| result.name == name)
}

fn check_ratio(
    results: &[BenchResult],
    faster_name: &str,
    slower_name: &str,
    max_ratio: f64,
    failures: &mut Vec<String>,
) {
    let Some(faster) = benchmark_result(results, faster_name) else {
        failures.push(format!("missing benchmark result: {faster_name}"));
        return;
    };
    let Some(slower) = benchmark_result(results, slower_name) else {
        failures.push(format!("missing benchmark result: {slower_name}"));
        return;
    };
    let ratio = faster.ns_per_iter / slower.ns_per_iter.max(f64::EPSILON);
    if ratio > max_ratio {
        failures.push(format!(
            "{faster_name} regression: ratio {:.3} exceeds {:.3} vs {slower_name}",
            ratio, max_ratio
        ));
    }
}

fn check_max_ns(
    results: &[BenchResult],
    benchmark_name: &str,
    max_ns: f64,
    failures: &mut Vec<String>,
) {
    let Some(result) = benchmark_result(results, benchmark_name) else {
        failures.push(format!("missing benchmark result: {benchmark_name}"));
        return;
    };
    if result.ns_per_iter > max_ns {
        failures.push(format!(
            "{benchmark_name} regression: {:.2} ns/iter exceeds {:.2} ns/iter",
            result.ns_per_iter, max_ns
        ));
    }
}

fn run_regression_checks(results: &[BenchResult]) -> Result<(), String> {
    let mut failures = Vec::new();
    check_ratio(
        results,
        "html_select_cached_selector",
        "html_select_parse_each_time",
        SELECT_CACHED_RATIO_MAX,
        &mut failures,
    );
    check_ratio(
        results,
        "html_xpath_precompiled",
        "html_xpath_parse_each_time",
        XPATH_PRECOMPILED_RATIO_MAX,
        &mut failures,
    );
    check_max_ns(
        results,
        "scheduler_priority_heap_roundtrip_2048",
        SCHEDULER_PRIORITY_HEAP_NS_MAX,
        &mut failures,
    );

    if failures.is_empty() {
        return Ok(());
    }

    let mut output = String::from("Benchmark regression thresholds failed:");
    for failure in failures {
        let _ = write!(output, "\n- {failure}");
    }
    Err(output)
}

fn make_response(body: Bytes, content_type: Option<&str>) -> Response<()> {
    let mut headers = Headers::new();
    if let Some(content_type) = content_type {
        headers.insert("content-type".to_string(), content_type.to_string());
    }
    Response {
        url: "https://example.com/catalog/".to_string(),
        status: 200,
        headers,
        body,
        request: Request::new("https://example.com/catalog/"),
    }
}

fn sample_html_document(items: usize) -> String {
    let mut html = String::with_capacity(items * 180);
    html.push_str(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>bench</title></head><body><section id=\"catalog\"><ul>",
    );
    for i in 0..items {
        let price = i * 7;
        let _ = write!(
            html,
            "<li class=\"item\" data-id=\"{i}\"><a class=\"link\" href=\"/items/{i}\">Item {i}</a><span class=\"price\">{price}</span><p class=\"desc\">Description {i}</p></li>"
        );
    }
    html.push_str("</ul></section></body></html>");
    html
}

fn sample_meta_encoded_html(words: usize) -> Bytes {
    let mut body = Vec::with_capacity(words * 8 + 80);
    body.extend_from_slice(b"<html><head><meta charset=\"windows-1252\"></head><body>");
    for i in 0..words {
        if i > 0 {
            body.push(b' ');
        }
        body.extend_from_slice(b"caf\xe9");
    }
    body.extend_from_slice(b"</body></html>");
    Bytes::from(body)
}

fn main() {
    let mut results = Vec::new();

    let template_request = Request::<()>::get("https://example.com/search")
        .with_headers([
            ("Accept", "text/html"),
            ("User-Agent", "silkworm-bench/1.0"),
            ("X-Trace", "bench"),
        ])
        .with_params([("q", "rust"), ("page", "1"), ("sort", "new")])
        .with_meta_str("trace_id", "abc123")
        .with_meta_bool("allow_non_html", false)
        .with_priority(10);

    results.push(run_bench("request_builder_fluent", || {
        Request::<()>::builder("https://example.com/search")
            .header("Accept", "text/html")
            .header("User-Agent", "silkworm-bench/1.0")
            .header("X-Trace", "bench")
            .param("q", "rust")
            .param("page", "1")
            .param("sort", "new")
            .allow_non_html(false)
            .dont_filter(true)
            .priority(10)
            .build()
    }));

    results.push(run_bench("request_clone", || template_request.clone()));

    let utf8_body = Bytes::from(sample_html_document(180).into_bytes());
    let utf8_response = make_response(utf8_body, Some("text/html; charset=utf-8"));
    results.push(run_bench("response_text_utf8", || utf8_response.text()));

    let meta_encoded_response = make_response(sample_meta_encoded_html(256), None);
    results.push(run_bench("response_text_meta_charset", || {
        meta_encoded_response.text()
    }));

    let follow_response = make_response(Bytes::new(), Some("text/html; charset=utf-8"));
    let hrefs = (0..1_000).map(|i| format!("/page/{i}")).collect::<Vec<_>>();
    results.push(run_bench("response_follow_urls_1000", || {
        follow_response.follow_urls(hrefs.iter())
    }));

    let html_bytes = Bytes::from(sample_html_document(350).into_bytes());
    results.push(run_bench("html_select_parse_each_time", || {
        let html = make_response(html_bytes.clone(), Some("text/html; charset=utf-8"))
            .into_html(HTML_MAX_SIZE_BYTES);
        html.select(".item .link").unwrap_or_default()
    }));

    let cached_html = make_response(html_bytes.clone(), Some("text/html; charset=utf-8"))
        .into_html(HTML_MAX_SIZE_BYTES);
    let selector = scraper::Selector::parse(".item .link").expect("valid selector");
    results.push(run_bench("html_select_cached_selector", || {
        cached_html.select_with(&selector)
    }));

    results.push(run_bench("html_xpath_parse_each_time", || {
        let html = make_response(html_bytes.clone(), Some("text/html; charset=utf-8"))
            .into_html(HTML_MAX_SIZE_BYTES);
        html.xpath("//li[@class='item']/a[@class='link']")
            .unwrap_or_default()
    }));

    let xpath = Factory::new()
        .build("//li[@class='item']/a[@class='link']")
        .expect("valid xpath")
        .expect("non-empty xpath");
    results.push(run_bench("html_xpath_precompiled", || {
        cached_html.xpath_with(&xpath).unwrap_or_default()
    }));

    results.push(run_bench("scheduler_priority_heap_roundtrip_2048", || {
        let mut queue = BinaryHeap::with_capacity(2_048);
        for i in 0..2_048u64 {
            queue.push(SchedulerEntry {
                priority: ((i % 9) as i32) - 4,
                sequence: i,
            });
        }
        let mut checksum = 0u64;
        while let Some(entry) = queue.pop() {
            checksum = checksum.wrapping_add(entry.sequence);
        }
        checksum
    }));

    render_results(&results);

    if benchmark_check_enabled()
        && let Err(message) = run_regression_checks(&results)
    {
        eprintln!("{message}");
        std::process::exit(1);
    }
}
