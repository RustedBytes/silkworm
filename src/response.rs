use std::cmp::min;
use std::collections::HashMap;
use std::fmt;
use std::ops::{Deref, DerefMut};

use encoding_rs::Encoding;
use regex::Regex;
use sxd_document::dom::{Attribute, Element};
use sxd_xpath::nodeset::Node;
use sxd_xpath::{Context, Factory, Value};
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
            Ok(base) => base
                .join(href)
                .map(|u| u.to_string())
                .unwrap_or_else(|_| href.to_string()),
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
            .flatten()
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
        let selector = scraper::Selector::parse(selector)
            .map_err(|err| SilkwormError::Selector(format!("Invalid CSS selector: {err}")))?;
        let source = self.html_source();
        let doc = scraper::Html::parse_document(&source);
        let mut out = Vec::new();
        for element in doc.select(&selector) {
            out.push(HtmlElement {
                html: element.html(),
                attrs: element_attrs(&element),
            });
        }
        Ok(out)
    }

    pub fn select_first(&self, selector: &str) -> SilkwormResult<Option<HtmlElement>> {
        let selector = scraper::Selector::parse(selector)
            .map_err(|err| SilkwormError::Selector(format!("Invalid CSS selector: {err}")))?;
        let source = self.html_source();
        let doc = scraper::Html::parse_document(&source);
        if let Some(element) = doc.select(&selector).next() {
            Ok(Some(HtmlElement {
                html: element.html(),
                attrs: element_attrs(&element),
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

    pub fn xpath(&self, selector: &str) -> SilkwormResult<Vec<HtmlElement>> {
        xpath_from_source(&self.html_source(), selector)
    }

    pub fn xpath_first(&self, selector: &str) -> SilkwormResult<Option<HtmlElement>> {
        Ok(xpath_from_source(&self.html_source(), selector)?
            .into_iter()
            .next())
    }

    fn html_source(&self) -> String {
        let limit = self.doc_max_size_bytes;
        if limit == 0 {
            return String::new();
        }
        let len = self.inner.body.len();
        let slice = if len > limit {
            &self.inner.body[..limit]
        } else {
            &self.inner.body
        };
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
    attrs: HashMap<String, String>,
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
        self.html.clone()
    }

    pub fn attr(&self, name: &str) -> Option<String> {
        if let Some(value) = self.attrs.get(name) {
            return Some(value.clone());
        }

        let doc = scraper::Html::parse_fragment(&self.html);
        let selector = scraper::Selector::parse("*").ok()?;
        for element in doc.select(&selector) {
            if let Some(value) = element.value().attr(name) {
                return Some(value.to_string());
            }
        }
        None
    }

    pub fn select(&self, selector: &str) -> SilkwormResult<Vec<HtmlElement>> {
        Self::select_from_source(&self.html, selector)
    }

    pub fn select_first(&self, selector: &str) -> SilkwormResult<Option<HtmlElement>> {
        Self::select_first_from_source(&self.html, selector)
    }

    fn select_from_source(source: &str, selector: &str) -> SilkwormResult<Vec<HtmlElement>> {
        let selector = scraper::Selector::parse(selector)
            .map_err(|err| SilkwormError::Selector(format!("Invalid CSS selector: {err}")))?;
        let doc = scraper::Html::parse_fragment(source);
        let mut out = Vec::new();
        for element in doc.select(&selector) {
            out.push(HtmlElement {
                html: element.html(),
                attrs: element_attrs(&element),
            });
        }
        Ok(out)
    }

    fn select_first_from_source(
        source: &str,
        selector: &str,
    ) -> SilkwormResult<Option<HtmlElement>> {
        let selector = scraper::Selector::parse(selector)
            .map_err(|err| SilkwormError::Selector(format!("Invalid CSS selector: {err}")))?;
        let doc = scraper::Html::parse_fragment(source);
        if let Some(element) = doc.select(&selector).next() {
            Ok(Some(HtmlElement {
                html: element.html(),
                attrs: element_attrs(&element),
            }))
        } else {
            Ok(None)
        }
    }
}

fn xpath_from_source(source: &str, selector: &str) -> SilkwormResult<Vec<HtmlElement>> {
    let package = sxd_html::parse_html(source);
    let document = package.as_document();

    let factory = Factory::new();
    let xpath = factory
        .build(selector)
        .map_err(|err| SilkwormError::Selector(format!("Invalid XPath selector: {err}")))?;
    let xpath = xpath.ok_or_else(|| {
        SilkwormError::Selector("Invalid XPath selector: empty expression".to_string())
    })?;

    let context = Context::new();
    let value = xpath
        .evaluate(&context, document.root())
        .map_err(|err| SilkwormError::Selector(format!("Invalid XPath selector: {err}")))?;

    Ok(xpath_value_to_elements(value))
}

fn xpath_value_to_elements(value: Value<'_>) -> Vec<HtmlElement> {
    match value {
        Value::Nodeset(nodeset) => nodeset
            .document_order()
            .into_iter()
            .map(|node| HtmlElement {
                html: xpath_node_to_html(node),
                attrs: HashMap::new(),
            })
            .collect(),
        Value::Boolean(value) => vec![HtmlElement {
            html: value.to_string(),
            attrs: HashMap::new(),
        }],
        Value::Number(value) => vec![HtmlElement {
            html: value.to_string(),
            attrs: HashMap::new(),
        }],
        Value::String(value) => vec![HtmlElement {
            html: value,
            attrs: HashMap::new(),
        }],
    }
}

fn xpath_node_to_html(node: Node<'_>) -> String {
    match node {
        Node::Root(_) | Node::Element(_) => node_to_markup(node),
        _ => node.string_value(),
    }
}

fn element_to_html(element: Element<'_>) -> String {
    let mut out = String::new();
    out.push('<');
    push_element_name(&mut out, element);

    for attribute in element.attributes() {
        out.push(' ');
        push_attribute_name(&mut out, attribute);
        out.push_str("=\"");
        out.push_str(&escape_attr(attribute.value()));
        out.push('"');
    }

    out.push('>');
    for child in Node::Element(element).children() {
        out.push_str(&node_to_markup(child));
    }
    out.push_str("</");
    push_element_name(&mut out, element);
    out.push('>');
    out
}

fn element_attrs(element: &scraper::ElementRef<'_>) -> HashMap<String, String> {
    let mut attrs = HashMap::new();
    for (name, value) in element.value().attrs.iter() {
        attrs.insert(name.local.to_string(), value.to_string());
    }
    attrs
}

fn node_to_markup(node: Node<'_>) -> String {
    match node {
        Node::Root(root) => Node::Root(root)
            .children()
            .into_iter()
            .map(node_to_markup)
            .collect(),
        Node::Element(element) => element_to_html(element),
        Node::Attribute(attribute) => escape_attr(attribute.value()),
        Node::Text(text) => escape_text(text.text()),
        Node::Comment(comment) => format!("<!--{}-->", comment.text()),
        Node::ProcessingInstruction(pi) => match pi.value() {
            Some(value) if !value.is_empty() => format!("<?{} {}?>", pi.target(), value),
            _ => format!("<?{}?>", pi.target()),
        },
        Node::Namespace(namespace) => namespace.uri().to_string(),
    }
}

fn push_element_name(out: &mut String, element: Element<'_>) {
    if let Some(prefix) = element.preferred_prefix() {
        out.push_str(prefix);
        out.push(':');
    }
    out.push_str(element.name().local_part());
}

fn push_attribute_name(out: &mut String, attribute: Attribute<'_>) {
    if let Some(prefix) = attribute.preferred_prefix() {
        out.push_str(prefix);
        out.push(':');
    }
    out.push_str(attribute.name().local_part());
}

fn escape_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

fn escape_attr(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
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

    let meta_charset = Regex::new(r#"(?i)<meta\s+charset\s*=\s*['"]?([a-zA-Z0-9._:-]+)"#).ok()?;
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
