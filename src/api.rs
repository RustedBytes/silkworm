use std::sync::OnceLock;
use std::time::Duration;

use crate::errors::SilkwormResult;
use crate::http::HttpClient;
use crate::request::Request;
use crate::types::Headers;

const API_DEFAULT_CONCURRENCY: usize = 8;
const API_DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
const API_DEFAULT_MAX_SIZE_BYTES: usize = 2_000_000;
const API_DEFAULT_MAX_REDIRECTS: usize = 10;

#[derive(Clone, Debug, PartialEq)]
pub struct UtilityFetchOptions {
    pub concurrency: usize,
    pub default_headers: Headers,
    pub timeout: Option<Duration>,
    pub html_max_size_bytes: usize,
    pub follow_redirects: bool,
    pub max_redirects: usize,
    pub keep_alive: bool,
}

impl Default for UtilityFetchOptions {
    fn default() -> Self {
        UtilityFetchOptions {
            concurrency: API_DEFAULT_CONCURRENCY,
            default_headers: Headers::new(),
            timeout: Some(API_DEFAULT_TIMEOUT),
            html_max_size_bytes: API_DEFAULT_MAX_SIZE_BYTES,
            follow_redirects: true,
            max_redirects: API_DEFAULT_MAX_REDIRECTS,
            keep_alive: false,
        }
    }
}

impl UtilityFetchOptions {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = concurrency;
        self
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.default_headers.insert(key.into(), value.into());
        self
    }

    pub fn with_headers<I, K, V>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (key, value) in headers {
            self.default_headers.insert(key.into(), value.into());
        }
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn without_timeout(mut self) -> Self {
        self.timeout = None;
        self
    }

    pub fn with_html_max_size_bytes(mut self, html_max_size_bytes: usize) -> Self {
        self.html_max_size_bytes = html_max_size_bytes;
        self
    }

    pub fn with_follow_redirects(mut self, follow_redirects: bool) -> Self {
        self.follow_redirects = follow_redirects;
        self
    }

    pub fn with_max_redirects(mut self, max_redirects: usize) -> Self {
        self.max_redirects = max_redirects;
        self
    }

    pub fn with_keep_alive(mut self, keep_alive: bool) -> Self {
        self.keep_alive = keep_alive;
        self
    }
}

#[derive(Clone)]
pub struct UtilityFetcher {
    client: HttpClient,
}

impl UtilityFetcher {
    pub fn new(options: UtilityFetchOptions) -> SilkwormResult<Self> {
        let client = if is_default_utility_options(&options) {
            shared_default_client()?
        } else {
            build_client(&options)?
        };
        Ok(UtilityFetcher { client })
    }

    pub async fn fetch_text(&self, url: &str) -> SilkwormResult<String> {
        fetch_text_with_client(&self.client, url).await
    }

    pub async fn fetch_html(&self, url: &str) -> SilkwormResult<(String, scraper::Html)> {
        let text = self.fetch_text(url).await?;
        let document = scraper::Html::parse_document(&text);
        Ok((text, document))
    }

    pub async fn fetch_document(&self, url: &str) -> SilkwormResult<scraper::Html> {
        let text = self.fetch_text(url).await?;
        Ok(scraper::Html::parse_document(&text))
    }
}

fn is_default_utility_options(options: &UtilityFetchOptions) -> bool {
    options == &UtilityFetchOptions::default()
}

fn build_client(options: &UtilityFetchOptions) -> SilkwormResult<HttpClient> {
    HttpClient::new(
        options.concurrency,
        options.default_headers.clone(),
        options.timeout,
        options.html_max_size_bytes,
        options.follow_redirects,
        options.max_redirects,
        options.keep_alive,
    )
}

fn shared_default_client() -> SilkwormResult<HttpClient> {
    static CLIENT: OnceLock<HttpClient> = OnceLock::new();
    let client = if let Some(existing) = CLIENT.get() {
        existing.clone()
    } else {
        let built = build_client(&UtilityFetchOptions::default())?;
        let _ = CLIENT.set(built.clone());
        built
    };
    Ok(client)
}

async fn fetch_text_with_options(
    url: &str,
    options: &UtilityFetchOptions,
) -> SilkwormResult<String> {
    let client = if is_default_utility_options(options) {
        shared_default_client()?
    } else {
        build_client(options)?
    };
    fetch_text_with_client(&client, url).await
}

async fn fetch_text_with_client(client: &HttpClient, url: &str) -> SilkwormResult<String> {
    let response = client.fetch(Request::<()>::get(url)).await?;
    Ok(response.text())
}

pub async fn fetch_html(url: &str) -> SilkwormResult<(String, scraper::Html)> {
    fetch_html_with(url, UtilityFetchOptions::default()).await
}

pub async fn fetch_html_with(
    url: &str,
    options: UtilityFetchOptions,
) -> SilkwormResult<(String, scraper::Html)> {
    let text = fetch_text_with_options(url, &options).await?;
    let document = scraper::Html::parse_document(&text);
    Ok((text, document))
}

pub async fn fetch_document(url: &str) -> SilkwormResult<scraper::Html> {
    fetch_document_with(url, UtilityFetchOptions::default()).await
}

pub async fn fetch_document_with(
    url: &str,
    options: UtilityFetchOptions,
) -> SilkwormResult<scraper::Html> {
    let text = fetch_text_with_options(url, &options).await?;
    Ok(scraper::Html::parse_document(&text))
}

#[cfg(test)]
mod tests {
    use super::{UtilityFetchOptions, UtilityFetcher, fetch_html, fetch_html_with};
    use crate::errors::SilkwormError;
    use scraper::Selector;
    use std::io::ErrorKind;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn start_test_server(
        body: &str,
    ) -> std::io::Result<(String, tokio::task::JoinHandle<()>)> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let body = body.to_string();
        let handle = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let mut request = Vec::new();
                loop {
                    let read = match socket.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(read) => read,
                    };
                    request.extend_from_slice(&buf[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        Ok((format!("http://{}", addr), handle))
    }

    async fn start_test_server_for_requests(
        body: &str,
        requests: usize,
    ) -> std::io::Result<(String, tokio::task::JoinHandle<()>)> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let body = body.to_string();
        let handle = tokio::spawn(async move {
            for _ in 0..requests {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = [0u8; 1024];
                let mut request = Vec::new();
                loop {
                    let read = match socket.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(read) => read,
                    };
                    request.extend_from_slice(&buf[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        Ok((format!("http://{}", addr), handle))
    }

    #[tokio::test]
    async fn fetch_html_returns_text_and_document() {
        let body = "<html><body><h1>Hello</h1></body></html>";
        let (url, handle) = match start_test_server(body).await {
            Ok(value) => value,
            Err(err) if err.kind() == ErrorKind::PermissionDenied => return,
            Err(err) => panic!("failed to start local test server: {err}"),
        };

        let (text, document) = fetch_html(&url).await.expect("fetch html");

        assert_eq!(text, body);
        let selector = Selector::parse("h1").expect("selector");
        let heading = document.select(&selector).next().expect("heading");
        assert_eq!(heading.text().collect::<String>(), "Hello");
        handle.await.expect("server task");
    }

    #[tokio::test]
    async fn fetch_html_with_respects_max_size_option() {
        let (url, handle) = match start_test_server("0123456789").await {
            Ok(value) => value,
            Err(err) if err.kind() == ErrorKind::PermissionDenied => return,
            Err(err) => panic!("failed to start local test server: {err}"),
        };

        let options = UtilityFetchOptions::new().with_html_max_size_bytes(4);
        let (text, _) = fetch_html_with(&url, options).await.expect("fetch html");

        assert_eq!(text, "0123");
        handle.await.expect("server task");
    }

    #[tokio::test]
    async fn utility_fetcher_reuses_custom_client() {
        let (url, handle) = match start_test_server_for_requests("0123456789", 2).await {
            Ok(value) => value,
            Err(err) if err.kind() == ErrorKind::PermissionDenied => return,
            Err(err) => panic!("failed to start local test server: {err}"),
        };

        let fetcher = UtilityFetcher::new(UtilityFetchOptions::new().with_html_max_size_bytes(4))
            .expect("utility fetcher");
        let first = fetcher.fetch_text(&url).await.expect("first fetch");
        let second = fetcher.fetch_text(&url).await.expect("second fetch");

        assert_eq!(first, "0123");
        assert_eq!(second, "0123");
        handle.await.expect("server task");
    }

    #[tokio::test]
    async fn fetch_html_with_rejects_zero_concurrency() {
        let options = UtilityFetchOptions::new().with_concurrency(0);
        let result = fetch_html_with("https://example.com", options).await;

        match result {
            Err(SilkwormError::Config(message)) => {
                assert!(message.contains("concurrency"));
            }
            Err(other) => panic!("expected config error, got {other:?}"),
            Ok(_) => panic!("expected error, got ok"),
        }
    }

    #[tokio::test]
    async fn fetch_html_invalid_url_returns_error() {
        let result = fetch_html("http://[::1").await;

        match result {
            Err(SilkwormError::Http(_)) => {}
            Ok(_) => panic!("expected error, got ok"),
            Err(other) => panic!("expected http error, got {other:?}"),
        }
    }
}
