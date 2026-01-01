use std::collections::{HashMap, HashSet};
use std::time::Duration;

use tokio::sync::Semaphore;
use url::Url;
use wreq::redirect::Policy;

use crate::errors::{SilkwormError, SilkwormResult};
use crate::logging::get_logger;
use crate::request::Request;
use crate::response::Response;
use crate::types::Headers;

#[derive(Clone)]
pub struct HttpClient {
    client: wreq::Client,
    semaphore: std::sync::Arc<Semaphore>,
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
        let mut visited: HashSet<String> = HashSet::new();

        loop {
            let url = self.build_url(&current_req)?;
            visited.insert(url.clone());

            let proxy = current_req
                .meta
                .get("proxy")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string());

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

            let response = match builder.send().await {
                Ok(resp) => resp,
                Err(err) => {
                    let msg = format!("Request to {} failed: {}", current_req.url, err);
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

                let location = headers.get("location").cloned().unwrap_or_default();
                let redirect_url = resolve_redirect_url(&url, &location);
                if visited.contains(&redirect_url) {
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

            let body = match response.bytes().await {
                Ok(bytes) => bytes.to_vec(),
                Err(err) => {
                    let msg = format!("Failed to read response body from {}: {}", url, err);
                    return Err(SilkwormError::Http(msg));
                }
            };

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

    pub async fn close(&self) {}

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

    fn build_url<S>(&self, req: &Request<S>) -> SilkwormResult<String> {
        if req.params.is_empty() {
            return Ok(req.url.clone());
        }
        let mut parsed = Url::parse(&req.url)
            .map_err(|err| SilkwormError::Http(format!("Invalid URL {}: {}", req.url, err)))?;

        let mut merged: HashMap<String, String> = parsed
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        for (key, value) in &req.params {
            merged.insert(key.clone(), value.clone());
        }

        parsed.query_pairs_mut().clear();
        {
            let mut pairs = parsed.query_pairs_mut();
            for (key, value) in merged {
                pairs.append_pair(&key, &value);
            }
        }

        Ok(parsed.to_string())
    }

    fn build_client_with_proxy(&self, proxy_url: &str) -> SilkwormResult<wreq::Client> {
        let proxy = wreq::Proxy::all(proxy_url)
            .map_err(|err| SilkwormError::Http(format!("Invalid proxy {}: {}", proxy_url, err)))?;
        let client = wreq::Client::builder()
            .redirect(Policy::none())
            .proxy(proxy)
            .build()?;
        Ok(client)
    }

    fn should_follow_redirect(&self, status: u16, headers: &Headers) -> bool {
        if !self.follow_redirects {
            return false;
        }
        matches!(status, 301 | 302 | 303 | 307 | 308) && headers.contains_key("location")
    }
}

fn resolve_redirect_url(current_url: &str, location: &str) -> String {
    if let Ok(base) = Url::parse(current_url) {
        if let Ok(joined) = base.join(location) {
            return joined.to_string();
        }
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

    let redirect_times = req
        .meta
        .get("redirect_times")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    req.meta.insert(
        "redirect_times".to_string(),
        serde_json::Value::from(redirect_times + 1),
    );

    req
}

fn normalize_headers(headers: &wreq::header::HeaderMap) -> Headers {
    let mut out = Headers::new();
    for (name, value) in headers.iter() {
        if let Ok(value) = value.to_str() {
            out.insert(name.to_string(), value.to_string());
        }
    }
    out
}
