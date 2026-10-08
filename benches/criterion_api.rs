//! Deterministic public-API workloads; see docs/benchmarks-criterion.md.
use std::fmt::Write as _;
use std::hint::black_box;

use bytes::Bytes;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use silkworm::{Headers, Request, Response};

const LIMIT: usize = 2_000_000;
const URL: &str = "https://example.com/catalog/";

fn response(body: Bytes, charset: &str) -> Response<()> {
    Response {
        url: URL.into(),
        status: 200,
        headers: Headers::from([("content-type".into(), charset.into())]),
        body,
        request: Request::get(URL),
    }
}

fn document(rows: usize) -> Bytes {
    let mut html = String::from("<html><body><ul>");
    for i in 0..rows {
        write!(
            html,
            "<li class=\"item\"><a href=\"/items/{i}\">Товар {i}</a><span>{}</span></li>",
            i * 7
        )
        .unwrap();
    }
    html.push_str("</ul></body></html>");
    Bytes::from(html)
}

fn requests(c: &mut Criterion) {
    let mut group = c.benchmark_group("request");
    group.throughput(Throughput::Elements(1));
    group.bench_function("builder", |b| {
        b.iter(|| {
            black_box(
                Request::<()>::builder(black_box(URL))
                    .header("Accept", "text/html")
                    .param("q", "rust")
                    .param("page", "1")
                    .priority(10)
                    .build(),
            )
        })
    });
    for count in [0, 8, 64] {
        let mut req = Request::<()>::get(URL);
        for i in 0..count {
            req = req.with_header(format!("X-Field-{i}"), format!("value-{i}"));
        }
        req.data = Some(Bytes::from(vec![42; 4096]));
        let copy = req.clone();
        assert_eq!(copy.headers, req.headers);
        assert_eq!(copy.data, req.data);
        group.bench_with_input(BenchmarkId::new("clone_headers", count), &req, |b, req| {
            b.iter(|| black_box(black_box(req).clone()));
        });
    }
    group.finish();
}

fn text(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode");
    for size in [1024, 65536, 1048576] {
        let utf8 = "Привіт café ".repeat(size / "Привіт café ".len());
        let latin = b"caf\xe9 ".repeat(size / 5);
        for (name, body, charset, expected) in [
            (
                "utf8",
                Bytes::from(utf8.clone()),
                "text/html; charset=utf-8",
                utf8,
            ),
            (
                "windows1252",
                Bytes::from(latin),
                "text/html; charset=windows-1252",
                "café ".repeat(size / 5),
            ),
        ] {
            let res = response(body, charset);
            assert_eq!(res.text(), expected);
            group.throughput(Throughput::Bytes(res.body.len() as u64));
            group.bench_with_input(BenchmarkId::new(name, res.body.len()), &res, |b, res| {
                b.iter(|| black_box(black_box(res).text()));
            });
        }
    }
    group.finish();
}

fn html(c: &mut Criterion) {
    let mut group = c.benchmark_group("html");
    let selector = scraper::Selector::parse(".item a").unwrap();
    #[cfg(feature = "xpath")]
    let queries = xee_xpath::Queries::new(xee_xpath::context::StaticContextBuilder::default());
    #[cfg(feature = "xpath")]
    let xpath = queries.sequence("//li[@class='item']/a").unwrap();
    for rows in [10, 200, 2000] {
        let bytes = document(rows);
        let warm = response(bytes.clone(), "text/html; charset=utf-8").into_html(LIMIT);
        assert_eq!(warm.select_with(&selector).len(), rows);
        assert_eq!(warm.select(".item a").unwrap().len(), rows);
        assert!(warm.select("[").is_err());
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("fresh_response_select", rows),
            &bytes,
            |b, bytes| {
                b.iter(|| {
                    let html = response(black_box(bytes).clone(), "text/html; charset=utf-8")
                        .into_html(LIMIT);
                    black_box(html.select_with(black_box(&selector)))
                });
            },
        );
        group.bench_function(BenchmarkId::new("warm_select_compiled", rows), |b| {
            b.iter(|| black_box(black_box(&warm).select_with(black_box(&selector))));
        });
        group.bench_function(BenchmarkId::new("warm_select_string", rows), |b| {
            b.iter(|| black_box(black_box(&warm).select(black_box(".item a")).unwrap()));
        });
        #[cfg(feature = "xpath")]
        {
            assert_eq!(warm.xpath_with(&xpath).unwrap().len(), rows);
            group.bench_function(BenchmarkId::new("xpath_compiled", rows), |b| {
                b.iter(|| black_box(black_box(&warm).xpath_with(black_box(&xpath)).unwrap()));
            });
            group.bench_function(BenchmarkId::new("xpath_string", rows), |b| {
                b.iter(|| {
                    black_box(
                        black_box(&warm)
                            .xpath(black_box("//li[@class='item']/a"))
                            .unwrap(),
                    )
                });
            });
        }
    }
    // Error handling has no bytes-processed interpretation.
    group.throughput(Throughput::Elements(1));
    let html = response(document(10), "text/html").into_html(LIMIT);
    group.bench_function("invalid_css", |b| {
        b.iter(|| black_box(html.select(black_box("["))));
    });
    group.finish();
}

fn links(c: &mut Criterion) {
    let mut group = c.benchmark_group("links");
    let res = response(Bytes::new(), "text/html");
    for count in [1, 100, 1000] {
        let hrefs: Vec<_> = (0..count)
            .map(|i| format!("../items/{i}?page=2#details"))
            .collect();
        let actual = res.follow_urls(&hrefs);
        assert_eq!(actual.len(), count);
        for (i, req) in actual.iter().enumerate() {
            assert_eq!(
                req.url,
                format!("https://example.com/items/{i}?page=2#details")
            );
        }
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::new("follow_urls", count),
            &hrefs,
            |b, hrefs| {
                b.iter(|| black_box(res.follow_urls(black_box(hrefs))));
            },
        );
    }
    group.finish();
}

criterion_group!(benches, requests, text, html, links);
criterion_main!(benches);
