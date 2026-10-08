#![forbid(unsafe_code)]

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use silkworm_parser_evaluation::{fixture, scraper_count, tl_count};
use std::hint::black_box;

fn compare(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser");
    let selector = scraper::Selector::parse("a[href]").unwrap();
    for rows in [10, 200, 2000] {
        let html = fixture(rows);
        assert_eq!(scraper_count(&html, "a[href]"), Some(rows));
        assert_eq!(tl_count(&html, "a[href]"), Some(rows));
        group.throughput(Throughput::Bytes(html.len() as u64));
        group.bench_with_input(BenchmarkId::new("scraper_parse", rows), &html, |b, html| {
            b.iter(|| black_box(scraper::Html::parse_document(black_box(html))));
        });
        group.bench_with_input(BenchmarkId::new("tl_parse", rows), &html, |b, html| {
            b.iter(|| black_box(tl::parse(black_box(html), tl::ParserOptions::default()).unwrap()));
        });
        let scraper_doc = scraper::Html::parse_document(&html);
        let tl_doc = tl::parse(&html, tl::ParserOptions::default()).unwrap();
        // Both warm selection workloads include parsing a string selector and count only.
        group.bench_function(BenchmarkId::new("scraper_warm_select", rows), |b| {
            b.iter(|| {
                let query = scraper::Selector::parse(black_box("a[href]")).unwrap();
                black_box(scraper_doc.select(&query).count())
            });
        });
        group.bench_function(BenchmarkId::new("tl_warm_select", rows), |b| {
            b.iter(|| black_box(tl_doc.query_selector(black_box("a[href]")).unwrap().count()));
        });
        // Separately measure the existing compiled-selector API, which tl does not share.
        group.bench_function(BenchmarkId::new("scraper_warm_compiled", rows), |b| {
            b.iter(|| black_box(scraper_doc.select(black_box(&selector)).count()));
        });
    }
    group.finish();
}
criterion_group!(benches, compare);
criterion_main!(benches);
