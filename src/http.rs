use std::collections::HashMap;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use tokio::sync::Semaphore;
use url::Url;
use wreq::redirect::Policy;

use crate::errors::{SilkwormError, SilkwormResult};
use crate::logging::get_logger;
use crate::request::Request;
use crate::response::Response;
use crate::types::{Headers, Params};

#[derive(Clone)]
pub struct HttpClient {
    client: wreq::Client,
    semaphore: std::sync::Arc<Semaphore>,
    proxy_clients: std::sync::Arc<std::sync::Mutex<HashMap<String, wreq::Client>>>,
    pub concurrency: usize,
    default_headers: Headers,
    timeout: Option<Duration>,
    pub html_max_size_bytes: usize,
    follow_redirects: bool,
    max_redirects: usize,
    keep_alive: bool,
    logger: crate::logging::Logger,
}

impl HttpClient {
    /// Create a configured HTTP client used by the engine and utility fetch APIs.
    ///
    /// # Errors
    ///
    /// Returns an error when `concurrency` is zero or the underlying HTTP client
    /// cannot be constructed.
    pub fn new(
        concurrency: usize,
        default_headers: Headers,
        timeout: Option<Duration>,
        html_max_size_bytes: usize,
        follow_redirects: bool,
        max_redirects: usize,
        keep_alive: bool,
    ) -> SilkwormResult<Self> {
        if concurrency == 0 {
            return Err(SilkwormError::Config(
                "concurrency must be greater than zero".to_string(),
            ));
        }
        let client = wreq::Client::builder().redirect(Policy::none()).build()?;

        Ok(HttpClient {
            client,
            semaphore: std::sync::Arc::new(Semaphore::new(concurrency)),
            proxy_clients: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
            concurrency,
            default_headers,
            timeout,
            html_max_size_bytes,
            follow_redirects,
            max_redirects,
            keep_alive,
            logger: get_logger("http", None),
        })
    }

    /// Fetch a request, optionally following redirects and truncating large bodies.
    ///
    /// # Errors
    ///
    /// Returns an error when request execution fails, redirect handling fails, or
    /// reading the response body fails.
    pub async fn fetch<S: Send + Sync + 'static>(
        &self,
        req: Request<S>,
    ) -> SilkwormResult<Response<S>> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| SilkwormError::Http("HTTP client semaphore closed".to_string()))?;

        let mut current_req = req;
        let mut redirects_followed = 0usize;
        let mut visited = Vec::with_capacity(self.max_redirects.saturating_add(1));

        loop {
            let url = Self::build_url(&current_req)?;
            visited.push(url.clone());

            let proxy = current_req.proxy().map(str::to_string);

            let client = if let Some(proxy_url) = &proxy {
                self.build_client_with_proxy(proxy_url)?
            } else {
                self.client.clone()
            };

            let method_name = current_req.method.to_uppercase();
            let method = wreq::Method::from_bytes(method_name.as_bytes())
                .map_err(|_| SilkwormError::Http("Invalid HTTP method".to_string()))?;

            let mut builder = client.request(method, &url);
            let headers = self.merge_headers(&current_req.headers);
            for (key, value) in headers {
                builder = builder.header(&key, value);
            }

            if let Some(timeout) = current_req.timeout.or(self.timeout) {
                builder = builder.timeout(timeout);
            }

            if let Some(json) = &current_req.json {
                builder = builder.json(json);
            } else if let Some(data) = &current_req.data {
                builder = builder.body(data.clone());
            }

            let mut response = match builder.send().await {
                Ok(resp) => resp,
                Err(err) => {
                    let msg = format!("Request to {} failed: {err}", current_req.url);
                    return Err(SilkwormError::Http(msg));
                }
            };

            let status = response.status().as_u16();
            let headers = normalize_headers(response.headers());

            if self.should_follow_redirect(status, &headers) {
                if redirects_followed >= self.max_redirects {
                    return Err(SilkwormError::Http(format!(
                        "Exceeded maximum redirects ({})",
                        self.max_redirects
                    )));
                }

                let location = header_value_case_insensitive(&headers, "location")
                    .map(str::to_string)
                    .unwrap_or_default();
                let redirect_url = resolve_redirect_url(&url, &location);
                if visited.iter().any(|seen| seen == &redirect_url) {
                    return Err(SilkwormError::Http("Redirect loop detected".to_string()));
                }

                redirects_followed += 1;
                self.logger.debug(
                    "Following redirect",
                    &[
                        ("from", url.clone()),
                        ("to", redirect_url.clone()),
                        ("status", status.to_string()),
                    ],
                );
                current_req = redirect_request(current_req, &redirect_url, status);
                continue;
            }

            let (body, truncated) =
                read_response_body_limited(&mut response, self.html_max_size_bytes, url.as_str())
                    .await?;
            if truncated {
                self.logger.warn(
                    "Response body truncated",
                    &[
                        ("url", url.clone()),
                        ("max_bytes", self.html_max_size_bytes.to_string()),
                    ],
                );
            }

            self.logger.debug(
                "HTTP response",
                &[
                    ("url", url.clone()),
                    ("status", status.to_string()),
                    ("proxy", proxy.is_some().to_string()),
                    ("redirects", redirects_followed.to_string()),
                ],
            );

            return Ok(Response {
                url,
                status,
                headers,
                body,
                request: current_req,
            });
        }
    }

    pub fn close(&self) {}

    fn merge_headers(&self, request_headers: &Headers) -> Headers {
        let mut headers = self.default_headers.clone();
        for (key, value) in request_headers {
            headers.insert(key.clone(), value.clone());
        }
        if self.keep_alive {
            let has_connection = headers
                .keys()
                .any(|key| key.eq_ignore_ascii_case("connection"));
            if !has_connection {
                headers.insert("Connection".to_string(), "keep-alive".to_string());
            }
        }
        headers
    }

    fn build_url<S>(req: &Request<S>) -> SilkwormResult<String> {
        build_url_with_params(&req.url, &req.params)
    }

    fn build_client_with_proxy(&self, proxy_url: &str) -> SilkwormResult<wreq::Client> {
        if let Ok(guard) = self.proxy_clients.lock()
            && let Some(client) = guard.get(proxy_url)
        {
            return Ok(client.clone());
        }
        let proxy = wreq::Proxy::all(proxy_url)
            .map_err(|err| SilkwormError::Http(format!("Invalid proxy {proxy_url}: {err}")))?;
        let client = wreq::Client::builder()
            .redirect(Policy::none())
            .proxy(proxy)
            .build()?;
        if let Ok(mut guard) = self.proxy_clients.lock() {
            guard
                .entry(proxy_url.to_string())
                .or_insert_with(|| client.clone());
        }
        Ok(client)
    }

    fn should_follow_redirect(&self, status: u16, headers: &Headers) -> bool {
        if !self.follow_redirects {
            return false;
        }
        matches!(status, 301 | 302 | 303 | 307 | 308)
            && header_value_case_insensitive(headers, "location").is_some()
    }
}

pub(crate) fn build_url_with_params(url: &str, params: &Params) -> SilkwormResult<String> {
    if params.is_empty() {
        let mut parsed = Url::parse(url)
            .map_err(|err| SilkwormError::Http(format!("Invalid URL {url}: {err}")))?;
        parsed.set_fragment(None);
        return Ok(parsed.to_string());
    }
    let mut parsed =
        Url::parse(url).map_err(|err| SilkwormError::Http(format!("Invalid URL {url}: {err}")))?;
    parsed.set_fragment(None);

    let merged = merged_query_pairs(&parsed, params);

    parsed.query_pairs_mut().clear();
    {
        let mut pairs = parsed.query_pairs_mut();
        for (key, value) in merged {
            pairs.append_pair(&key, &value);
        }
    }

    Ok(parsed.to_string())
}

pub(crate) fn canonical_url_with_params(url: &str, params: &Params) -> SilkwormResult<String> {
    let mut parsed =
        Url::parse(url).map_err(|err| SilkwormError::Http(format!("Invalid URL {url}: {err}")))?;
    parsed.set_fragment(None);
    let mut merged = merged_query_pairs(&parsed, params);
    merged.sort();

    parsed.query_pairs_mut().clear();
    {
        let mut pairs = parsed.query_pairs_mut();
        for (key, value) in merged {
            pairs.append_pair(&key, &value);
        }
    }

    Ok(parsed.to_string())
}

fn merged_query_pairs(url: &Url, params: &Params) -> Vec<(String, String)> {
    let mut merged: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    if params.is_empty() {
        return merged;
    }

    merged.retain(|(key, _)| !params.contains_key(key));

    // HashMap iteration order is nondeterministic; keep appended params stable.
    let mut additions: Vec<(String, String)> = params
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    additions.sort();
    merged.extend(additions);

    merged
}

async fn read_response_body_limited(
    response: &mut wreq::Response,
    max_bytes: usize,
    url: &str,
) -> SilkwormResult<(Bytes, bool)> {
    if max_bytes == 0 {
        while response
            .chunk()
            .await
            .map_err(|err| {
                SilkwormError::Http(format!("Failed to read response body from {url}: {err}"))
            })?
            .is_some()
        {}
        return Ok((Bytes::new(), true));
    }

    let mut body = BytesMut::with_capacity(max_bytes.min(8192));
    let mut truncated = false;
    loop {
        let chunk = response.chunk().await.map_err(|err| {
            SilkwormError::Http(format!("Failed to read response body from {url}: {err}"))
        })?;
        let Some(chunk) = chunk else { break };

        if body.len() >= max_bytes {
            truncated = true;
            continue;
        }

        let remaining = max_bytes - body.len();
        if chunk.len() <= remaining {
            body.extend_from_slice(&chunk);
        } else {
            body.extend_from_slice(&chunk[..remaining]);
            truncated = true;
        }
    }

    Ok((body.freeze(), truncated))
}

fn resolve_redirect_url(current_url: &str, location: &str) -> String {
    if let Ok(base) = Url::parse(current_url)
        && let Ok(joined) = base.join(location)
    {
        return joined.to_string();
    }
    location.to_string()
}

fn redirect_request<S>(mut req: Request<S>, redirect_url: &str, status: u16) -> Request<S> {
    let method_name = req.method.to_uppercase();
    if matches!(status, 301..=303) && !matches!(method_name.as_str(), "GET" | "HEAD") {
        req.method = "GET".to_string();
        req.data = None;
        req.json = None;
    }
    req.url = redirect_url.to_string();
    req.params.clear();

    req.increment_redirect_times();

    req
}

fn normalize_headers(headers: &wreq::header::HeaderMap) -> Headers {
    let mut out = Headers::new();
    for (name, value) in headers {
        if let Ok(value) = value.to_str() {
            let key = name.to_string().to_ascii_lowercase();
            if let Some(existing) = out.get_mut(&key) {
                existing.push_str(", ");
                existing.push_str(value);
            } else {
                out.insert(key, value.to_string());
            }
        }
    }
    out
}

fn header_value_case_insensitive<'a>(headers: &'a Headers, name: &str) -> Option<&'a str> {
    let normalized = name.to_ascii_lowercase();
    if let Some(value) = headers.get(&normalized) {
        return Some(value.as_str());
    }
    if let Some(value) = headers.get(name) {
        return Some(value.as_str());
    }
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

#[cfg(test)]
mod tests {
    use super::{
        HttpClient, canonical_url_with_params, normalize_headers, redirect_request,
        resolve_redirect_url,
    };
    use crate::request::Request;
    use crate::types::{Headers, Item};
    use std::collections::HashMap;
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
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        Ok((format!("http://{addr}"), handle))
    }

    #[test]
    fn build_url_merges_params() {
        let req = Request::<()>::new("https://example.com/path?foo=bar")
            .with_param("foo", "baz")
            .with_param("q", "1");
        let built = HttpClient::build_url(&req).expect("build url");
        let parsed = url::Url::parse(&built).expect("parse url");
        let query: HashMap<String, String> = parsed.query_pairs().into_owned().collect();
        assert_eq!(query.get("foo"), Some(&"baz".to_string()));
        assert_eq!(query.get("q"), Some(&"1".to_string()));
    }

    #[test]
    fn build_url_preserves_repeated_query_keys() {
        let req =
            Request::<()>::new("https://example.com/path?tag=a&tag=b&x=1").with_param("q", "1");
        let built = HttpClient::build_url(&req).expect("build url");
        let parsed = url::Url::parse(&built).expect("parse url");
        let tags: Vec<String> = parsed
            .query_pairs()
            .filter_map(|(k, v)| (k == "tag").then(|| v.into_owned()))
            .collect();

        assert_eq!(tags, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn build_url_strips_fragment() {
        let req = Request::<()>::new("https://example.com/path?x=1#section");
        let built = HttpClient::build_url(&req).expect("build url");

        assert!(!built.contains('#'));
        assert_eq!(built, "https://example.com/path?x=1");
    }

    #[test]
    fn canonical_url_sorts_query_pairs_without_dropping_duplicates() {
        let params = [("z".to_string(), "9".to_string())]
            .into_iter()
            .collect::<crate::types::Params>();
        let canonical = canonical_url_with_params("https://example.com/path?b=2&a=1&b=1", &params)
            .expect("canonical");
        let parsed = url::Url::parse(&canonical).expect("parse canonical");
        let query: Vec<(String, String)> = parsed.query_pairs().into_owned().collect();

        assert_eq!(
            query,
            vec![
                ("a".to_string(), "1".to_string()),
                ("b".to_string(), "1".to_string()),
                ("b".to_string(), "2".to_string()),
                ("z".to_string(), "9".to_string())
            ]
        );
    }

    #[test]
    fn canonical_url_ignores_fragment() {
        let params = crate::types::Params::new();
        let first = canonical_url_with_params("https://example.com/path?x=1#top", &params)
            .expect("canonical #1");
        let second = canonical_url_with_params("https://example.com/path?x=1#bottom", &params)
            .expect("canonical #2");

        assert_eq!(first, second);
        assert_eq!(first, "https://example.com/path?x=1");
    }

    #[test]
    fn merge_headers_adds_keep_alive() {
        let client =
            HttpClient::new(1, Headers::new(), None, 1024, false, 3, true).expect("client");
        let request_headers = Headers::new();
        let merged = client.merge_headers(&request_headers);
        assert_eq!(
            merged.get("Connection").map(String::as_str),
            Some("keep-alive")
        );
    }

    #[test]
    fn resolve_redirect_url_joins_relative() {
        let resolved = resolve_redirect_url("https://example.com/path/page", "/next");
        assert_eq!(resolved, "https://example.com/next");
    }

    #[test]
    fn redirect_request_resets_method_and_payload() {
        let req = Request::<()>::new("https://example.com/original")
            .with_method("POST")
            .with_data(vec![1, 2, 3])
            .with_json(Item::from(1))
            .with_param("q", "1");
        let redirected = redirect_request(req, "https://example.com/new", 301);
        assert_eq!(redirected.method, "GET");
        assert!(redirected.data.is_none());
        assert!(redirected.json.is_none());
        assert_eq!(redirected.url, "https://example.com/new");
        assert!(redirected.params.is_empty());
        assert_eq!(redirected.redirect_times(), 1);
    }

    #[test]
    fn normalize_headers_copies_valid_entries() {
        let mut headers = wreq::header::HeaderMap::new();
        headers.insert(
            "Content-Type",
            wreq::header::HeaderValue::from_static("text/html"),
        );
        let normalized = normalize_headers(&headers);
        assert_eq!(
            normalized.get("content-type").map(String::as_str),
            Some("text/html")
        );
    }

    #[test]
    fn normalize_headers_merges_duplicate_values() {
        let mut headers = wreq::header::HeaderMap::new();
        headers.append(
            "cache-control",
            wreq::header::HeaderValue::from_static("no-cache"),
        );
        headers.append(
            "cache-control",
            wreq::header::HeaderValue::from_static("max-age=0"),
        );

        let normalized = normalize_headers(&headers);
        assert_eq!(
            normalized.get("cache-control").map(String::as_str),
            Some("no-cache, max-age=0")
        );
    }

    #[test]
    fn should_follow_redirect_is_case_insensitive_for_location() {
        let client =
            HttpClient::new(1, Headers::new(), None, 1024, true, 3, false).expect("client");
        let mut headers = Headers::new();
        headers.insert(
            "Location".to_string(),
            "https://example.com/next".to_string(),
        );
        assert!(client.should_follow_redirect(302, &headers));
    }

    #[tokio::test]
    async fn fetch_truncates_body_to_html_max_size_bytes() {
        let (url, handle) = match start_test_server("0123456789").await {
            Ok(value) => value,
            Err(err) if err.kind() == ErrorKind::PermissionDenied => return,
            Err(err) => panic!("failed to start local test server: {err}"),
        };

        let client = HttpClient::new(1, Headers::new(), None, 4, false, 3, false).expect("client");
        let response = client
            .fetch(Request::<()>::new(url.clone()))
            .await
            .expect("response");

        let normalized_url = url::Url::parse(&url).expect("parse url").to_string();
        assert_eq!(response.url, normalized_url);
        assert_eq!(response.body.as_ref(), b"0123");
        handle.await.expect("server task");
    }
}
