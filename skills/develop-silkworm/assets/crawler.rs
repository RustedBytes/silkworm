use silkworm::*;
use std::{sync::Arc, time::Duration};

pub struct Catalog {
    pub seed: String,
}
impl Spider for Catalog {
    async fn start_requests(&self) -> Vec<Request<Self>> {
        vec![Request::get(self.seed.clone())]
    }
    async fn parse(&self, response: HtmlResponse<Self>) -> SpiderResult<Self> {
        if !response.status_ok() {
            return Err(SilkwormError::Http(format!(
                "unexpected status {}",
                response.status
            )));
        }
        let mut out = Vec::new();
        for entry in response.select("article")? {
            let title = entry.text_from("h2");
            if !title.trim().is_empty() {
                out.push(SpiderOutput::Item(serde_json::json!({"title": title})));
            }
        }
        if let Some(link) = response.select_first("a.next")?
            && let Some(href) = link.attr("href")
        {
            let request = response.follow_url(&href);
            if let (Ok(seed), Ok(next)) =
                (url::Url::parse(&self.seed), url::Url::parse(&request.url))
                && seed.origin() == next.origin()
            {
                out.push(request.into());
            }
        }
        Ok(out)
    }
}
pub struct Identify;
impl<S: Spider> RequestMiddleware<S> for Identify {
    fn process_request<'a>(
        &'a self,
        request: Request<S>,
        _: Arc<S>,
    ) -> MiddlewareFuture<'a, Request<S>> {
        Box::pin(async move { request.with_header("X-Crawler", "catalog") })
    }
}
pub struct Observe;
impl<S: Spider> ResponseMiddleware<S> for Observe {
    fn process_response<'a>(
        &'a self,
        response: Response<S>,
        spider: Arc<S>,
    ) -> MiddlewareFuture<'a, ResponseAction<S>> {
        Box::pin(async move {
            spider
                .log()
                .debug("received", &[("status", response.status.to_string())]);
            ResponseAction::Response(response)
        })
    }
}
pub struct Enrich;
impl<S: Spider> ItemPipeline<S> for Enrich {
    fn open<'a>(&'a self, _: Arc<S>) -> PipelineFuture<'a, SilkwormResult<()>> {
        Box::pin(async { Ok(()) })
    }
    fn close<'a>(&'a self, _: Arc<S>) -> PipelineFuture<'a, SilkwormResult<()>> {
        Box::pin(async { Ok(()) })
    }
    fn process_item<'a>(
        &'a self,
        mut item: Item,
        spider: Arc<S>,
    ) -> PipelineFuture<'a, SilkwormResult<Item>> {
        Box::pin(async move {
            if let Some(map) = item.as_object_mut() {
                map.insert("spider".into(), serde_json::json!(spider.name()));
            }
            Ok(item)
        })
    }
}
pub async fn crawl_catalog(seed: String, output: &str) -> SilkwormResult<()> {
    let config = RunConfig::new()
        .with_concurrency(4)
        .with_max_pending_requests(100)
        .with_max_seen_requests(1000)
        .with_html_max_size_bytes(512_000)
        .with_request_timeout(Duration::from_secs(10))
        .with_log_stats_interval(Duration::from_secs(10))
        .with_fail_fast(true)
        .with_request_middleware(Identify)
        .with_response_middleware(RetryMiddleware::new(2, None, None, 0.5))
        .with_response_middleware(Observe)
        .with_item_pipeline(Enrich)
        .with_item_pipeline(JsonLinesPipeline::new(output));
    crawl_with(Catalog { seed }, config).await
}
fn fixture(body: &'static [u8], status: u16) -> Response<Catalog> {
    Response {
        url: "http://localhost/catalog".into(),
        status,
        headers: Headers::new(),
        body: bytes::Bytes::from_static(body),
        request: Request::get("http://localhost/catalog"),
    }
}
#[tokio::test]
async fn extracts_items_and_scopes_pagination() {
    let spider = Catalog {
        seed: "http://localhost/catalog".into(),
    };
    let outputs = spider
        .parse(
            fixture(
                b"<article><h2>Ada</h2></article><a class='next' href='/page/2'>Next</a>",
                200,
            )
            .into_html(512_000),
        )
        .await
        .unwrap();
    assert_eq!(outputs.len(), 2);
    match &outputs[0] {
        SpiderOutput::Item(item) => assert_eq!(item["title"], "Ada"),
        _ => panic!("item"),
    }
    match &outputs[1] {
        SpiderOutput::Request(req) => assert_eq!(req.url, "http://localhost/page/2"),
        _ => panic!("request"),
    }
    let outputs = spider
        .parse(
            fixture(
                b"<article><h2> </h2></article><a class='next' href='http://other.test/'>Next</a>",
                200,
            )
            .into_html(512_000),
        )
        .await
        .unwrap();
    assert!(outputs.is_empty());
    assert!(
        spider
            .parse(fixture(b"", 503).into_html(512_000))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn extensions_and_retry_contract() {
    let spider = Arc::new(Catalog {
        seed: "http://localhost/catalog".into(),
    });
    let request = Identify
        .process_request(Request::get(&spider.seed), spider.clone())
        .await;
    assert_eq!(request.headers.get("X-Crawler").unwrap(), "catalog");
    let item = Enrich
        .process_item(serde_json::json!({"title":"Ada"}), spider.clone())
        .await
        .unwrap();
    assert_eq!(item["spider"], "spider");
    let response = Observe
        .process_response(fixture(b"", 200), spider.clone())
        .await;
    assert!(matches!(response, ResponseAction::Response(_)));
    let retry = RetryMiddleware::new(1, None, None, 0.0);
    let action = retry
        .process_response(fixture(b"", 503), spider.clone())
        .await;
    let ResponseAction::Request(request) = action else {
        panic!("retry")
    };
    assert!(request.dont_filter);
    assert_eq!(request.retry_times(), 1);
    let mut exhausted = fixture(b"", 503);
    exhausted.request = request;
    assert!(matches!(
        retry.process_response(exhausted, spider).await,
        ResponseAction::Response(_)
    ));
}
#[cfg(feature = "xpath")]
#[test]
fn xpath_extracts_fixture() {
    let response = fixture(b"<article><h2>Ada</h2></article>", 200).into_html(512_000);
    assert_eq!(response.xpath("//h2").unwrap()[0].text(), "Ada");
}

#[tokio::test]
async fn local_crawl_flushes_enriched_jsonl() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(pair) => break pair,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "server accept timeout"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("{error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            assert_eq!(stream.read(&mut byte).unwrap(), 1);
            request.push(byte[0]);
            assert!(request.len() < 16384);
        }
        assert!(
            String::from_utf8(request)
                .unwrap()
                .to_ascii_lowercase()
                .contains("x-crawler: catalog")
        );
        let body = "<article><h2>Ada</h2></article>";
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let path = std::env::temp_dir().join(format!(
        "silkworm-skill-{}-{}.jsonl",
        std::process::id(),
        address.port()
    ));
    let result = crawl_catalog(format!("http://{address}/"), path.to_str().unwrap()).await;
    server.join().unwrap();
    result.unwrap();
    let data = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    let items: Vec<serde_json::Value> = data
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        items,
        vec![serde_json::json!({"title":"Ada", "spider":"spider"})]
    );
}
