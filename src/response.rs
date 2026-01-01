use std::cmp::min;
use std::fmt;
use std::ops::{Deref, DerefMut};

use encoding_rs::Encoding;
use regex::Regex;
use url::Url;

use crate::errors::{SilkwormError, SilkwormResult};
use crate::request::{Callback, Request};
use crate::types::Headers;

#[derive(Clone)]
pub struct Response<S> {
    pub url: String,
    pub status: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
    pub request: Request<S>,
}

impl<S> fmt::Debug for Response<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Response")
            .field("url", &self.url)
            .field("status", &self.status)
            .field("headers", &self.headers)
            .field("body_len", &self.body.len())
            .field("request", &self.request)
            .finish()
    }
}

impl<S> Response<S> {
    pub fn text(&self) -> String {
        decode_body(&self.body, &self.headers).0
    }

    pub fn encoding(&self) -> String {
        decode_body(&self.body, &self.headers).1
    }

    pub fn url_join(&self, href: &str) -> String {
        match Url::parse(&self.url) {
            Ok(base) => base.join(href).map(|u| u.to_string()).unwrap_or_else(|_| href.to_string()),
            Err(_) => href.to_string(),
        }
    }

    pub fn follow(&self, href: &str, callback: Option<Callback<S>>) -> Request<S> {
        let url = self.url_join(href);
        let mut req = Request::new(url);
        req.callback = callback.or_else(|| self.request.callback.clone());
        req
    }

    pub fn follow_all<I, H>(&self, hrefs: I, callback: Option<Callback<S>>) -> Vec<Request<S>>
    where
        I: IntoIterator<Item = Option<H>>,
        H: AsRef<str>,
    {
        hrefs
            .into_iter()
            .filter_map(|href| href)
            .map(|href| self.follow(href.as_ref(), callback.clone()))
            .collect()
    }

    pub fn looks_like_html(&self) -> bool {
        let content_type = self
            .headers
            .get("content-type")
            .map(|value| value.to_lowercase())
            .unwrap_or_default();

        if content_type.contains("html") {
            return true;
        }

        let sniff_len = min(2048, self.body.len());
        let snippet = &self.body[..sniff_len];
        if snippet.is_empty() {
            return false;
        }

        let snippet_lower = String::from_utf8_lossy(snippet).to_lowercase();
        snippet_lower.contains("<html") || snippet_lower.contains("<!doctype")
    }

    pub fn close(&mut self) {
        self.body.clear();
        self.headers.clear();
    }

    pub fn into_html(self, doc_max_size_bytes: usize) -> HtmlResponse<S> {
        HtmlResponse {
            inner: self,
            doc_max_size_bytes,
        }
    }
}

#[derive(Clone)]
pub struct HtmlResponse<S> {
    inner: Response<S>,
    pub doc_max_size_bytes: usize,
}

impl<S> HtmlResponse<S> {
    pub fn select(&self, selector: &str) -> SilkwormResult<Vec<HtmlElement>> {
        let selector = scraper::Selector::parse(selector).map_err(|err| {
            SilkwormError::Selector(format!("Invalid CSS selector: {err}"))
        })?;
        let source = self.html_source();
        let doc = scraper::Html::parse_document(&source);
        let mut out = Vec::new();
        for element in doc.select(&selector) {
            out.push(HtmlElement {
                html: element.html(),
            });
        }
        Ok(out)
    }

    pub fn select_first(&self, selector: &str) -> SilkwormResult<Option<HtmlElement>> {
        let selector = scraper::Selector::parse(selector).map_err(|err| {
            SilkwormError::Selector(format!("Invalid CSS selector: {err}"))
        })?;
        let source = self.html_source();
        let doc = scraper::Html::parse_document(&source);
        if let Some(element) = doc.select(&selector).next() {
            Ok(Some(HtmlElement {
                html: element.html(),
            }))
        } else {
            Ok(None)
        }
    }

    pub fn css(&self, selector: &str) -> SilkwormResult<Vec<HtmlElement>> {
        self.select(selector)
    }

    pub fn css_first(&self, selector: &str) -> SilkwormResult<Option<HtmlElement>> {
        self.select_first(selector)
    }

    pub fn xpath(&self, _selector: &str) -> SilkwormResult<Vec<HtmlElement>> {
        Err(SilkwormError::Selector("XPath selectors are not supported".to_string()))
    }

    pub fn xpath_first(&self, _selector: &str) -> SilkwormResult<Option<HtmlElement>> {
        Err(SilkwormError::Selector("XPath selectors are not supported".to_string()))
    }

    fn html_source(&self) -> String {
        let limit = self.doc_max_size_bytes;
        if limit == 0 {
            return String::new();
        }
        let len = self.inner.body.len();
        let slice = if len > limit { &self.inner.body[..limit] } else { &self.inner.body };
        decode_body(slice, &self.inner.headers).0
    }
}

impl<S> Deref for HtmlResponse<S> {
    type Target = Response<S>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<S> DerefMut for HtmlResponse<S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

#[derive(Clone)]
pub struct HtmlElement {
    html: String,
}

impl HtmlElement {
    pub fn html(&self) -> &str {
        &self.html
    }

    pub fn text(&self) -> String {
        let doc = scraper::Html::parse_fragment(&self.html);
        let selector = scraper::Selector::parse("*").ok();
        if let Some(selector) = selector {
            if let Some(element) = doc.select(&selector).next() {
                return element.text().collect::<Vec<_>>().join("");
            }
        }
        String::new()
    }

    pub fn attr(&self, name: &str) -> Option<String> {
        let doc = scraper::Html::parse_fragment(&self.html);
        let selector = scraper::Selector::parse("*").ok()?;
        let element = doc.select(&selector).next()?;
        element.value().attr(name).map(|value| value.to_string())
    }

    pub fn select(&self, selector: &str) -> SilkwormResult<Vec<HtmlElement>> {
        Self::select_from_source(&self.html, selector)
    }

    pub fn select_first(&self, selector: &str) -> SilkwormResult<Option<HtmlElement>> {
        Self::select_first_from_source(&self.html, selector)
    }

    fn select_from_source(source: &str, selector: &str) -> SilkwormResult<Vec<HtmlElement>> {
        let selector = scraper::Selector::parse(selector).map_err(|err| {
            SilkwormError::Selector(format!("Invalid CSS selector: {err}"))
        })?;
        let doc = scraper::Html::parse_fragment(source);
        let mut out = Vec::new();
        for element in doc.select(&selector) {
            out.push(HtmlElement {
                html: element.html(),
            });
        }
        Ok(out)
    }

    fn select_first_from_source(source: &str, selector: &str) -> SilkwormResult<Option<HtmlElement>> {
        let selector = scraper::Selector::parse(selector).map_err(|err| {
            SilkwormError::Selector(format!("Invalid CSS selector: {err}"))
        })?;
        let doc = scraper::Html::parse_fragment(source);
        if let Some(element) = doc.select(&selector).next() {
            Ok(Some(HtmlElement {
                html: element.html(),
            }))
        } else {
            Ok(None)
        }
    }
}

fn decode_body(body: &[u8], headers: &Headers) -> (String, String) {
    if body.is_empty() {
        return (String::new(), "utf-8".to_string());
    }

    let mut candidates = Vec::new();
    if let Some(enc) = encoding_from_bom(body) {
        candidates.push(enc);
    }
    if let Some(enc) = encoding_from_headers(headers) {
        candidates.push(enc);
    }
    if let Some(enc) = encoding_from_meta(body) {
        candidates.push(enc);
    }

    let mut seen = std::collections::HashSet::new();
    for enc in candidates {
        let normalized = enc.trim().to_lowercase();
        if normalized.is_empty() || seen.contains(&normalized) {
            continue;
        }
        seen.insert(normalized.clone());
        if let Some((text, label)) = decode_with_label(body, &normalized) {
            return (text, label);
        }
    }

    (
        String::from_utf8_lossy(body).to_string(),
        "utf-8".to_string(),
    )
}

fn decode_with_label(body: &[u8], label: &str) -> Option<(String, String)> {
    let encoding = Encoding::for_label(label.as_bytes())?;
    let (text, _, _) = encoding.decode(body);
    Some((text.into_owned(), encoding.name().to_string()))
}

fn encoding_from_headers(headers: &Headers) -> Option<String> {
    let content_type = headers.get("content-type")?;
    let re = Regex::new(r#"(?i)charset=([^"'\s;]+)"#).ok()?;
    let caps = re.captures(content_type)?;
    caps.get(1).map(|m| m.as_str().to_string())
}

fn encoding_from_meta(body: &[u8]) -> Option<String> {
    let head_len = min(4096, body.len());
    let head = String::from_utf8_lossy(&body[..head_len]);

    let meta_charset =
        Regex::new(r#"(?i)<meta\s+charset\s*=\s*['"]?([a-zA-Z0-9._:-]+)"#).ok()?;
    if let Some(caps) = meta_charset.captures(&head) {
        return caps.get(1).map(|m| m.as_str().to_string());
    }

    let meta_content_type = Regex::new(
        r#"(?i)<meta\s+http-equiv\s*=\s*['"]?content-type['"]?[^>]*charset\s*=\s*['"]?([a-zA-Z0-9._:-]+)"#,
    )
    .ok()?;
    if let Some(caps) = meta_content_type.captures(&head) {
        return caps.get(1).map(|m| m.as_str().to_string());
    }

    let xml_decl =
        Regex::new(r#"(?i)<\?xml[^>]+encoding\s*=\s*['"]([a-zA-Z0-9._:-]+)['"]"#).ok()?;
    xml_decl
        .captures(&head)
        .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))
}

fn encoding_from_bom(body: &[u8]) -> Option<String> {
    if body.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Some("utf-8".to_string());
    }
    if body.starts_with(&[0xFF, 0xFE, 0x00, 0x00]) {
        return Some("utf-32le".to_string());
    }
    if body.starts_with(&[0x00, 0x00, 0xFE, 0xFF]) {
        return Some("utf-32be".to_string());
    }
    if body.starts_with(&[0xFF, 0xFE]) {
        return Some("utf-16le".to_string());
    }
    if body.starts_with(&[0xFE, 0xFF]) {
        return Some("utf-16be".to_string());
    }
    None
}
