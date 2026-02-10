use std::cmp::min;
use std::collections::HashMap;
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::sync::OnceLock;

use bytes::Bytes;
use encoding_rs::Encoding;
use regex::Regex;
#[cfg(feature = "xpath")]
use sxd_document::dom::{Attribute, Element};
#[cfg(feature = "xpath")]
use sxd_xpath::nodeset::Node;
#[cfg(feature = "xpath")]
use sxd_xpath::{Context, Factory, Value};
use url::Url;

use crate::errors::{SilkwormError, SilkwormResult};
use crate::request::{Callback, Request, SpiderOutput};
use crate::types::Headers;

#[derive(Clone)]
pub struct Response<S> {
    pub url: String,
    pub status: u16,
    pub headers: Headers,
    pub body: Bytes,
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
        decode_body(self.body.as_ref(), &self.headers).0
    }

    pub fn encoding(&self) -> String {
        decode_body(self.body.as_ref(), &self.headers).1
    }

    pub fn status_ok(&self) -> bool {
        (200..=299).contains(&self.status)
    }

    pub fn is_redirect(&self) -> bool {
        matches!(self.status, 301 | 302 | 303 | 307 | 308)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        if let Some(value) = self.headers.get(name) {
            return Some(value.as_str());
        }
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    #[inline]
    pub fn content_type(&self) -> Option<&str> {
        self.header("content-type")
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

    #[inline]
    pub fn follow_url(&self, href: &str) -> Request<S> {
        self.follow(href, None)
    }

    pub fn follow_urls<I, H>(&self, hrefs: I) -> Vec<Request<S>>
    where
        I: IntoIterator<Item = H>,
        H: AsRef<str>,
    {
        hrefs
            .into_iter()
            .filter_map(|href| {
                let href = href.as_ref();
                if href.is_empty() {
                    None
                } else {
                    Some(self.follow_url(href))
                }
            })
            .collect()
    }

    pub fn follow_urls_outputs<I, H>(&self, hrefs: I) -> Vec<SpiderOutput<S>>
    where
        I: IntoIterator<Item = H>,
        H: AsRef<str>,
    {
        self.follow_urls(hrefs)
            .into_iter()
            .map(Into::into)
            .collect()
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

    /// Follow multiple URLs, filtering out empty strings.
    /// This is a convenience method for cases where you have an iterator of strings
    /// rather than Option<String>.
    pub fn follow_many<I, H>(&self, hrefs: I, callback: Option<Callback<S>>) -> Vec<Request<S>>
    where
        I: IntoIterator<Item = H>,
        H: AsRef<str>,
    {
        hrefs
            .into_iter()
            .filter(|href| !href.as_ref().is_empty())
            .map(|href| self.follow(href.as_ref(), callback.clone()))
            .collect()
    }

    pub fn looks_like_html(&self) -> bool {
        let content_type = self
            .content_type()
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
        contains_ascii_case_insensitive(snippet, b"<html")
            || contains_ascii_case_insensitive(snippet, b"<!doctype")
    }

    pub fn close(&mut self) {
        self.body = Bytes::new();
        self.headers.clear();
    }

    #[cfg(feature = "scraper-atomic")]
    pub fn into_html(self, doc_max_size_bytes: usize) -> HtmlResponse<S> {
        HtmlResponse {
            inner: self,
            doc_max_size_bytes,
            cached_source: OnceLock::new(),
            cached_document: OnceLock::new(),
        }
    }

    #[cfg(not(feature = "scraper-atomic"))]
    pub fn into_html(self, doc_max_size_bytes: usize) -> HtmlResponse<S> {
        HtmlResponse {
            inner: self,
            doc_max_size_bytes,
            cached_source: OnceLock::new(),
        }
    }
}

pub struct HtmlResponse<S> {
    inner: Response<S>,
    pub doc_max_size_bytes: usize,
    cached_source: OnceLock<String>,
    #[cfg(feature = "scraper-atomic")]
    cached_document: OnceLock<scraper::Html>,
}

#[cfg(feature = "scraper-atomic")]
impl<S: Clone> Clone for HtmlResponse<S> {
    fn clone(&self) -> Self {
        HtmlResponse {
            inner: self.inner.clone(),
            doc_max_size_bytes: self.doc_max_size_bytes,
            cached_source: OnceLock::new(),
            cached_document: OnceLock::new(),
        }
    }
}

#[cfg(not(feature = "scraper-atomic"))]
impl<S: Clone> Clone for HtmlResponse<S> {
    fn clone(&self) -> Self {
        HtmlResponse {
            inner: self.inner.clone(),
            doc_max_size_bytes: self.doc_max_size_bytes,
            cached_source: OnceLock::new(),
        }
    }
}

impl<S> HtmlResponse<S> {
    pub fn select(&self, selector: &str) -> SilkwormResult<Vec<HtmlElement>> {
        let selector = scraper::Selector::parse(selector)
            .map_err(|err| SilkwormError::Selector(format!("Invalid CSS selector: {err}")))?;
        Ok(self.select_with(&selector))
    }

    pub fn select_first(&self, selector: &str) -> SilkwormResult<Option<HtmlElement>> {
        let selector = scraper::Selector::parse(selector)
            .map_err(|err| SilkwormError::Selector(format!("Invalid CSS selector: {err}")))?;
        Ok(self.select_first_with(&selector))
    }

    #[cfg(feature = "scraper-atomic")]
    pub fn select_with(&self, selector: &scraper::Selector) -> Vec<HtmlElement> {
        let doc = self.document();
        let mut out = Vec::new();
        for element in doc.select(selector) {
            out.push(HtmlElement {
                html: element.html(),
                attrs: element_attrs(&element),
            });
        }
        out
    }

    #[cfg(not(feature = "scraper-atomic"))]
    pub fn select_with(&self, selector: &scraper::Selector) -> Vec<HtmlElement> {
        let doc = scraper::Html::parse_document(self.html_source());
        let mut out = Vec::new();
        for element in doc.select(selector) {
            out.push(HtmlElement {
                html: element.html(),
                attrs: element_attrs(&element),
            });
        }
        out
    }

    #[cfg(feature = "scraper-atomic")]
    pub fn select_first_with(&self, selector: &scraper::Selector) -> Option<HtmlElement> {
        let doc = self.document();
        doc.select(selector).next().map(|element| HtmlElement {
            html: element.html(),
            attrs: element_attrs(&element),
        })
    }

    #[cfg(not(feature = "scraper-atomic"))]
    pub fn select_first_with(&self, selector: &scraper::Selector) -> Option<HtmlElement> {
        let doc = scraper::Html::parse_document(self.html_source());
        doc.select(selector).next().map(|element| HtmlElement {
            html: element.html(),
            attrs: element_attrs(&element),
        })
    }

    #[cfg(feature = "scraper-atomic")]
    fn select_texts_with(&self, selector: &scraper::Selector) -> Vec<String> {
        let doc = self.document();
        doc.select(selector)
            .map(|element| element.text().collect::<String>())
            .collect()
    }

    #[cfg(not(feature = "scraper-atomic"))]
    fn select_texts_with(&self, selector: &scraper::Selector) -> Vec<String> {
        let doc = scraper::Html::parse_document(self.html_source());
        doc.select(selector)
            .map(|element| element.text().collect::<String>())
            .collect()
    }

    #[cfg(feature = "scraper-atomic")]
    fn select_attrs_with(&self, selector: &scraper::Selector, attr_name: &str) -> Vec<String> {
        let doc = self.document();
        doc.select(selector)
            .filter_map(|element| element.value().attr(attr_name).map(str::to_string))
            .collect()
    }

    #[cfg(not(feature = "scraper-atomic"))]
    fn select_attrs_with(&self, selector: &scraper::Selector, attr_name: &str) -> Vec<String> {
        let doc = scraper::Html::parse_document(self.html_source());
        doc.select(selector)
            .filter_map(|element| element.value().attr(attr_name).map(str::to_string))
            .collect()
    }

    #[inline]
    pub fn css(&self, selector: &str) -> SilkwormResult<Vec<HtmlElement>> {
        self.select(selector)
    }

    #[inline]
    pub fn css_first(&self, selector: &str) -> SilkwormResult<Option<HtmlElement>> {
        self.select_first(selector)
    }

    pub fn xpath(&self, selector: &str) -> SilkwormResult<Vec<HtmlElement>> {
        #[cfg(feature = "xpath")]
        {
            xpath_from_source(self.html_source(), selector)
        }

        #[cfg(not(feature = "xpath"))]
        {
            let _ = selector;
            Err(SilkwormError::Selector(
                "XPath support is disabled; enable the `xpath` feature".to_string(),
            ))
        }
    }

    pub fn xpath_first(&self, selector: &str) -> SilkwormResult<Option<HtmlElement>> {
        self.xpath(selector).map(|nodes| nodes.into_iter().next())
    }

    #[cfg(feature = "xpath")]
    pub fn xpath_with(&self, xpath: &sxd_xpath::XPath) -> SilkwormResult<Vec<HtmlElement>> {
        xpath_from_source_with(self.html_source(), xpath)
    }

    #[cfg(feature = "xpath")]
    pub fn xpath_first_with(
        &self,
        xpath: &sxd_xpath::XPath,
    ) -> SilkwormResult<Option<HtmlElement>> {
        Ok(xpath_from_source_with(self.html_source(), xpath)?
            .into_iter()
            .next())
    }

    /// Select elements matching a CSS selector, returning an empty vector on error.
    /// This is a convenience method that ignores selector parsing errors.
    #[inline]
    pub fn select_or_empty(&self, selector: &str) -> Vec<HtmlElement> {
        self.select(selector).unwrap_or_default()
    }

    /// Select the first element matching a CSS selector, returning None on error.
    /// This is a convenience method that ignores selector parsing errors.
    #[inline]
    pub fn select_first_or_none(&self, selector: &str) -> Option<HtmlElement> {
        self.select_first(selector).ok().flatten()
    }

    /// Select elements matching a CSS selector, returning an empty vector on error.
    /// Alias for `select_or_empty`.
    #[inline]
    pub fn css_or_empty(&self, selector: &str) -> Vec<HtmlElement> {
        self.select_or_empty(selector)
    }

    /// Select the first element matching a CSS selector, returning None on error.
    /// Alias for `select_first_or_none`.
    #[inline]
    pub fn css_first_or_none(&self, selector: &str) -> Option<HtmlElement> {
        self.select_first_or_none(selector)
    }

    /// Select elements matching an XPath selector, returning an empty vector on error.
    /// This is a convenience method that ignores selector parsing errors.
    #[inline]
    pub fn xpath_or_empty(&self, selector: &str) -> Vec<HtmlElement> {
        self.xpath(selector).unwrap_or_default()
    }

    /// Select the first element matching an XPath selector, returning None on error.
    /// This is a convenience method that ignores selector parsing errors.
    #[inline]
    pub fn xpath_first_or_none(&self, selector: &str) -> Option<HtmlElement> {
        self.xpath_first(selector).ok().flatten()
    }

    /// Select elements matching a CSS selector and return their text.
    pub fn select_texts(&self, selector: &str) -> Vec<String> {
        let selector = match scraper::Selector::parse(selector) {
            Ok(selector) => selector,
            Err(_) => return Vec::new(),
        };
        self.select_texts_with(&selector)
    }

    /// Select elements matching a CSS selector and return the requested attribute values.
    pub fn select_attrs(&self, selector: &str, attr_name: &str) -> Vec<String> {
        let selector = match scraper::Selector::parse(selector) {
            Ok(selector) => selector,
            Err(_) => return Vec::new(),
        };
        self.select_attrs_with(&selector, attr_name)
    }

    /// Select the first element matching a CSS selector and return its text.
    /// Returns an empty string if the selector doesn't match or fails to parse.
    pub fn text_from(&self, selector: &str) -> String {
        self.select_first_or_none(selector)
            .map(|el| el.text())
            .unwrap_or_default()
    }

    /// Select the first element matching a CSS selector and return an attribute value.
    /// Returns None if the selector doesn't match, fails to parse, or the attribute doesn't exist.
    pub fn attr_from(&self, selector: &str, attr_name: &str) -> Option<String> {
        self.select_first_or_none(selector)
            .and_then(|el| el.attr(attr_name))
    }

    /// Follow links extracted from a CSS selector and attribute name.
    #[inline]
    pub fn follow_css(&self, selector: &str, attr_name: &str) -> Vec<Request<S>> {
        self.follow_urls(self.select_attrs(selector, attr_name))
    }

    /// Follow links extracted from a CSS selector and attribute name, returning outputs.
    #[inline]
    pub fn follow_css_outputs(&self, selector: &str, attr_name: &str) -> Vec<SpiderOutput<S>> {
        self.follow_urls_outputs(self.select_attrs(selector, attr_name))
    }

    fn html_source(&self) -> &str {
        self.cached_source
            .get_or_init(|| {
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
            })
            .as_str()
    }

    #[cfg(feature = "scraper-atomic")]
    fn document(&self) -> &scraper::Html {
        self.cached_document
            .get_or_init(|| scraper::Html::parse_document(self.html_source()))
    }

    #[cfg(feature = "scraper-atomic")]
    fn clear_cache(&mut self) {
        self.cached_source = OnceLock::new();
        self.cached_document = OnceLock::new();
    }

    #[cfg(not(feature = "scraper-atomic"))]
    fn clear_cache(&mut self) {
        self.cached_source = OnceLock::new();
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
        self.clear_cache();
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
        if let Some(selector) = select_all_selector()
            && let Some(element) = doc.select(selector).next()
        {
            return element.text().collect::<String>();
        }
        self.html.clone()
    }

    pub fn attr(&self, name: &str) -> Option<String> {
        if let Some(value) = self.attrs.get(name) {
            return Some(value.clone());
        }

        let doc = scraper::Html::parse_fragment(&self.html);
        if let Some(selector) = select_all_selector() {
            for element in doc.select(selector) {
                if let Some(value) = element.value().attr(name) {
                    return Some(value.to_string());
                }
            }
        }
        None
    }

    #[inline]
    pub fn select(&self, selector: &str) -> SilkwormResult<Vec<HtmlElement>> {
        Self::select_from_source(&self.html, selector)
    }

    #[inline]
    pub fn select_first(&self, selector: &str) -> SilkwormResult<Option<HtmlElement>> {
        Self::select_first_from_source(&self.html, selector)
    }

    #[inline]
    pub fn select_with(&self, selector: &scraper::Selector) -> Vec<HtmlElement> {
        Self::select_with_from_source(&self.html, selector)
    }

    #[inline]
    pub fn select_first_with(&self, selector: &scraper::Selector) -> Option<HtmlElement> {
        Self::select_first_with_from_source(&self.html, selector)
    }

    /// Select elements matching a CSS selector, returning an empty vector on error.
    /// This is a convenience method that ignores selector parsing errors.
    #[inline]
    pub fn select_or_empty(&self, selector: &str) -> Vec<HtmlElement> {
        self.select(selector).unwrap_or_default()
    }

    /// Select the first element matching a CSS selector, returning None on error.
    /// This is a convenience method that ignores selector parsing errors.
    #[inline]
    pub fn select_first_or_none(&self, selector: &str) -> Option<HtmlElement> {
        self.select_first(selector).ok().flatten()
    }

    /// Select the first element matching a CSS selector and return its text.
    /// Returns an empty string if the selector doesn't match or fails to parse.
    #[inline]
    pub fn text_from(&self, selector: &str) -> String {
        self.select_first_or_none(selector)
            .map(|el| el.text())
            .unwrap_or_default()
    }

    /// Select the first element matching a CSS selector and return an attribute value.
    /// Returns None if the selector doesn't match, fails to parse, or the attribute doesn't exist.
    #[inline]
    pub fn attr_from(&self, selector: &str, attr_name: &str) -> Option<String> {
        self.select_first_or_none(selector)
            .and_then(|el| el.attr(attr_name))
    }

    /// Select elements matching a CSS selector and return their text.
    #[inline]
    pub fn select_texts(&self, selector: &str) -> Vec<String> {
        Self::select_texts_from_source(&self.html, selector)
    }

    /// Select elements matching a CSS selector and return the requested attribute values.
    #[inline]
    pub fn select_attrs(&self, selector: &str, attr_name: &str) -> Vec<String> {
        Self::select_attrs_from_source(&self.html, selector, attr_name)
    }

    fn select_from_source(source: &str, selector: &str) -> SilkwormResult<Vec<HtmlElement>> {
        let selector = scraper::Selector::parse(selector)
            .map_err(|err| SilkwormError::Selector(format!("Invalid CSS selector: {err}")))?;
        Ok(Self::select_with_from_source(source, &selector))
    }

    fn select_first_from_source(
        source: &str,
        selector: &str,
    ) -> SilkwormResult<Option<HtmlElement>> {
        let selector = scraper::Selector::parse(selector)
            .map_err(|err| SilkwormError::Selector(format!("Invalid CSS selector: {err}")))?;
        Ok(Self::select_first_with_from_source(source, &selector))
    }

    fn select_with_from_source(source: &str, selector: &scraper::Selector) -> Vec<HtmlElement> {
        let doc = scraper::Html::parse_fragment(source);
        let mut out = Vec::new();
        for element in doc.select(selector) {
            out.push(HtmlElement {
                html: element.html(),
                attrs: element_attrs(&element),
            });
        }
        out
    }

    fn select_first_with_from_source(
        source: &str,
        selector: &scraper::Selector,
    ) -> Option<HtmlElement> {
        let doc = scraper::Html::parse_fragment(source);
        doc.select(selector).next().map(|element| HtmlElement {
            html: element.html(),
            attrs: element_attrs(&element),
        })
    }

    fn select_texts_from_source(source: &str, selector: &str) -> Vec<String> {
        let selector = match scraper::Selector::parse(selector) {
            Ok(selector) => selector,
            Err(_) => return Vec::new(),
        };
        let doc = scraper::Html::parse_fragment(source);
        doc.select(&selector)
            .map(|element| element.text().collect::<String>())
            .collect()
    }

    fn select_attrs_from_source(source: &str, selector: &str, attr_name: &str) -> Vec<String> {
        let selector = match scraper::Selector::parse(selector) {
            Ok(selector) => selector,
            Err(_) => return Vec::new(),
        };
        let doc = scraper::Html::parse_fragment(source);
        doc.select(&selector)
            .filter_map(|element| element.value().attr(attr_name).map(str::to_string))
            .collect()
    }
}

#[cfg(feature = "xpath")]
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

    xpath_from_source_with_document(document, &xpath)
}

#[cfg(feature = "xpath")]
fn xpath_from_source_with(
    source: &str,
    xpath: &sxd_xpath::XPath,
) -> SilkwormResult<Vec<HtmlElement>> {
    let package = sxd_html::parse_html(source);
    let document = package.as_document();
    xpath_from_source_with_document(document, xpath)
}

#[cfg(feature = "xpath")]
fn xpath_from_source_with_document(
    document: sxd_document::dom::Document<'_>,
    xpath: &sxd_xpath::XPath,
) -> SilkwormResult<Vec<HtmlElement>> {
    let context = Context::new();
    let value = xpath
        .evaluate(&context, document.root())
        .map_err(|err| SilkwormError::Selector(format!("Invalid XPath selector: {err}")))?;
    Ok(xpath_value_to_elements(value))
}

#[cfg(feature = "xpath")]
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

#[cfg(feature = "xpath")]
fn xpath_node_to_html(node: Node<'_>) -> String {
    match node {
        Node::Root(_) | Node::Element(_) => node_to_markup(node),
        _ => node.string_value(),
    }
}

#[cfg(feature = "xpath")]
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
    let raw_attrs = &element.value().attrs;
    let mut attrs = HashMap::with_capacity(raw_attrs.len());
    for (name, value) in raw_attrs.iter() {
        attrs.insert(name.local.to_string(), value.to_string());
    }
    attrs
}

fn select_all_selector() -> Option<&'static scraper::Selector> {
    static SELECT_ALL: OnceLock<Option<scraper::Selector>> = OnceLock::new();
    SELECT_ALL
        .get_or_init(|| scraper::Selector::parse("*").ok())
        .as_ref()
}

#[cfg(feature = "xpath")]
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

#[cfg(feature = "xpath")]
fn push_element_name(out: &mut String, element: Element<'_>) {
    if let Some(prefix) = element.preferred_prefix() {
        out.push_str(prefix);
        out.push(':');
    }
    out.push_str(element.name().local_part());
}

#[cfg(feature = "xpath")]
fn push_attribute_name(out: &mut String, attribute: Attribute<'_>) {
    if let Some(prefix) = attribute.preferred_prefix() {
        out.push_str(prefix);
        out.push(':');
    }
    out.push_str(attribute.name().local_part());
}

#[cfg(feature = "xpath")]
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

#[cfg(feature = "xpath")]
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

fn header_value_case_insensitive<'a>(headers: &'a Headers, name: &str) -> Option<&'a str> {
    if let Some(value) = headers.get(name) {
        return Some(value.as_str());
    }
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn encoding_from_headers(headers: &Headers) -> Option<String> {
    let content_type = header_value_case_insensitive(headers, "content-type")?;
    static CHARSET_RE: OnceLock<Regex> = OnceLock::new();
    let re =
        CHARSET_RE.get_or_init(|| Regex::new(r#"(?i)charset=([^"'\s;]+)"#).expect("charset regex"));
    let caps = re.captures(content_type)?;
    caps.get(1).map(|m| m.as_str().to_string())
}

fn encoding_from_meta(body: &[u8]) -> Option<String> {
    let head_len = min(4096, body.len());
    let head = String::from_utf8_lossy(&body[..head_len]);

    static META_CHARSET_RE: OnceLock<Regex> = OnceLock::new();
    let meta_charset = META_CHARSET_RE.get_or_init(|| {
        Regex::new(r#"(?i)<meta\s+charset\s*=\s*['"]?([a-zA-Z0-9._:-]+)"#)
            .expect("meta charset regex")
    });
    if let Some(caps) = meta_charset.captures(&head) {
        return caps.get(1).map(|m| m.as_str().to_string());
    }

    static META_CONTENT_TYPE_RE: OnceLock<Regex> = OnceLock::new();
    let meta_content_type = META_CONTENT_TYPE_RE.get_or_init(|| {
        Regex::new(
            r#"(?i)<meta\s+http-equiv\s*=\s*['"]?content-type['"]?[^>]*charset\s*=\s*['"]?([a-zA-Z0-9._:-]+)"#,
        )
        .expect("meta content-type regex")
    });
    if let Some(caps) = meta_content_type.captures(&head) {
        return caps.get(1).map(|m| m.as_str().to_string());
    }

    static XML_DECL_RE: OnceLock<Regex> = OnceLock::new();
    let xml_decl = XML_DECL_RE.get_or_init(|| {
        Regex::new(r#"(?i)<\?xml[^>]+encoding\s*=\s*['"]([a-zA-Z0-9._:-]+)['"]"#)
            .expect("xml declaration regex")
    });
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

#[inline]
fn contains_ascii_case_insensitive(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|window| {
        window
            .iter()
            .zip(needle.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

#[cfg(test)]
mod tests {
    use super::Response;
    use crate::request::{Request, SpiderOutput, SpiderResult, callback_from_fn};
    use crate::types::Headers;
    use bytes::Bytes;
    use std::sync::Arc;

    #[test]
    fn response_url_join_resolves_relative() {
        let request = Request::<()>::new("https://example.com");
        let response = Response {
            url: "https://example.com/path/".to_string(),
            status: 200,
            headers: Headers::new(),
            body: Bytes::new(),
            request,
        };
        assert_eq!(response.url_join("next"), "https://example.com/path/next");
    }

    #[test]
    fn follow_all_filters_none_and_joins_urls() {
        let request = Request::<()>::new("https://example.com/base/");
        let response = Response {
            url: "https://example.com/base/".to_string(),
            status: 200,
            headers: Headers::new(),
            body: Bytes::new(),
            request,
        };
        let hrefs = vec![Some("/one"), None, Some("two")];
        let requests = response.follow_all(hrefs, None);
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].url, "https://example.com/one");
        assert_eq!(requests[1].url, "https://example.com/base/two");
    }

    #[test]
    fn follow_url_preserves_callback() {
        async fn handler(_spider: Arc<()>, _response: Response<()>) -> SpiderResult<()> {
            Ok(Vec::new())
        }

        let callback = callback_from_fn(handler);
        let mut request = Request::<()>::new("https://example.com");
        request.callback = Some(callback);
        let response = Response {
            url: "https://example.com".to_string(),
            status: 200,
            headers: Headers::new(),
            body: Bytes::new(),
            request,
        };

        let next = response.follow_url("/next");
        assert!(next.callback.is_some());
        assert_eq!(next.url, "https://example.com/next");
    }

    #[test]
    fn looks_like_html_detects_content_type() {
        let mut headers = Headers::new();
        headers.insert("content-type".to_string(), "text/html".to_string());
        let response = Response {
            url: "https://example.com".to_string(),
            status: 200,
            headers,
            body: Bytes::new(),
            request: Request::<()>::new("https://example.com"),
        };
        assert!(response.looks_like_html());
    }

    #[test]
    fn status_helpers_report_success_and_redirects() {
        let response = Response {
            url: "https://example.com".to_string(),
            status: 204,
            headers: Headers::new(),
            body: Bytes::new(),
            request: Request::<()>::new("https://example.com"),
        };
        assert!(response.status_ok());
        assert!(!response.is_redirect());

        let redirect = Response {
            url: "https://example.com".to_string(),
            status: 302,
            headers: Headers::new(),
            body: Bytes::new(),
            request: Request::<()>::new("https://example.com"),
        };
        assert!(redirect.is_redirect());
    }

    #[test]
    fn header_and_content_type_helpers_handle_case() {
        let mut headers = Headers::new();
        headers.insert("Content-Type".to_string(), "text/html".to_string());
        headers.insert("X-Trace".to_string(), "abc".to_string());
        let response = Response {
            url: "https://example.com".to_string(),
            status: 200,
            headers,
            body: Bytes::new(),
            request: Request::<()>::new("https://example.com"),
        };
        assert_eq!(response.header("content-type"), Some("text/html"));
        assert_eq!(response.header("X-TRACE"), Some("abc"));
        assert_eq!(response.content_type(), Some("text/html"));
    }

    #[test]
    fn html_response_selects_and_reads_attributes() {
        let body = b"<html><body><a href=\"/x\">Link</a></body></html>";
        let mut headers = Headers::new();
        headers.insert("content-type".to_string(), "text/html".to_string());
        let response = Response {
            url: "https://example.com".to_string(),
            status: 200,
            headers,
            body: Bytes::from_static(body),
            request: Request::<()>::new("https://example.com"),
        };
        let html = response.into_html(1024);
        let anchor = html.select_first("a").expect("select").expect("anchor");
        assert_eq!(anchor.text(), "Link");
        assert_eq!(anchor.attr("href").as_deref(), Some("/x"));
        #[cfg(feature = "xpath")]
        {
            let xpath = html.xpath_first("//a").expect("xpath").expect("node");
            assert!(xpath.html().contains("<a"));
        }
    }

    #[test]
    fn close_clears_body_and_headers() {
        let mut headers = Headers::new();
        headers.insert("content-type".to_string(), "text/plain".to_string());
        let mut response = Response {
            url: "https://example.com".to_string(),
            status: 200,
            headers,
            body: Bytes::from_static(b"hi"),
            request: Request::<()>::new("https://example.com"),
        };
        response.close();
        assert!(response.body.is_empty());
        assert!(response.headers.is_empty());
    }

    #[test]
    fn select_or_empty_returns_empty_on_invalid_selector() {
        let body = b"<html><body><div class=\"test\">Content</div></body></html>";
        let response = Response {
            url: "https://example.com".to_string(),
            status: 200,
            headers: Headers::new(),
            body: Bytes::from_static(body),
            request: Request::<()>::new("https://example.com"),
        };
        let html = response.into_html(1024);

        // Invalid selector returns empty
        let results = html.select_or_empty("div[invalid");
        assert_eq!(results.len(), 0);

        // Valid selector works
        let results = html.select_or_empty(".test");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text(), "Content");
    }

    #[test]
    fn select_first_or_none_returns_none_on_invalid_selector() {
        let body = b"<html><body><div class=\"test\">Content</div></body></html>";
        let response = Response {
            url: "https://example.com".to_string(),
            status: 200,
            headers: Headers::new(),
            body: Bytes::from_static(body),
            request: Request::<()>::new("https://example.com"),
        };
        let html = response.into_html(1024);

        // Invalid selector returns None
        assert!(html.select_first_or_none("div[invalid").is_none());

        // Valid selector works
        let element = html.select_first_or_none(".test");
        assert!(element.is_some());
        assert_eq!(element.unwrap().text(), "Content");
    }

    #[test]
    fn text_from_extracts_text_from_selector() {
        let body = b"<html><body><h1>Title</h1><p class=\"content\">Text</p></body></html>";
        let response = Response {
            url: "https://example.com".to_string(),
            status: 200,
            headers: Headers::new(),
            body: Bytes::from_static(body),
            request: Request::<()>::new("https://example.com"),
        };
        let html = response.into_html(1024);

        assert_eq!(html.text_from("h1"), "Title");
        assert_eq!(html.text_from(".content"), "Text");
        assert_eq!(html.text_from(".missing"), "");
    }

    #[test]
    fn attr_from_extracts_attribute() {
        let body = b"<html><body><a href=\"/link\" class=\"test\">Link</a></body></html>";
        let response = Response {
            url: "https://example.com".to_string(),
            status: 200,
            headers: Headers::new(),
            body: Bytes::from_static(body),
            request: Request::<()>::new("https://example.com"),
        };
        let html = response.into_html(1024);

        assert_eq!(html.attr_from("a", "href"), Some("/link".to_string()));
        assert_eq!(html.attr_from("a", "class"), Some("test".to_string()));
        assert_eq!(html.attr_from("a", "missing"), None);
        assert_eq!(html.attr_from(".missing", "href"), None);
    }

    #[test]
    fn select_texts_and_attrs_collect_values() {
        let body = b"<html><body><ul><li>One</li><li>Two</li></ul><a href=\"/a\">A</a><a>Skip</a></body></html>";
        let response = Response {
            url: "https://example.com".to_string(),
            status: 200,
            headers: Headers::new(),
            body: Bytes::from_static(body),
            request: Request::<()>::new("https://example.com"),
        };
        let html = response.into_html(1024);

        assert_eq!(
            html.select_texts("li"),
            vec!["One".to_string(), "Two".to_string()]
        );
        assert_eq!(html.select_attrs("a", "href"), vec!["/a".to_string()]);

        let list = html.select_first_or_none("ul").expect("list");
        assert_eq!(
            list.select_texts("li"),
            vec!["One".to_string(), "Two".to_string()]
        );
    }

    #[test]
    fn follow_css_outputs_builds_requests() {
        let body = b"<html><body><a href=\"/next\">Next</a><a href=\"\"></a></body></html>";
        let response = Response {
            url: "https://example.com/base/".to_string(),
            status: 200,
            headers: Headers::new(),
            body: Bytes::from_static(body),
            request: Request::<()>::new("https://example.com/base/"),
        };
        let html = response.into_html(1024);
        let outputs = html.follow_css_outputs("a", "href");
        assert_eq!(outputs.len(), 1);
        match &outputs[0] {
            SpiderOutput::Request(req) => {
                assert_eq!(req.url, "https://example.com/next");
            }
            SpiderOutput::Item(_) => panic!("expected request output"),
        }
    }

    #[test]
    fn follow_many_filters_empty_strings() {
        let request = Request::<()>::new("https://example.com/base/");
        let response = Response {
            url: "https://example.com/base/".to_string(),
            status: 200,
            headers: Headers::new(),
            body: Bytes::new(),
            request,
        };
        let hrefs = vec!["/one", "", "two", ""];
        let requests = response.follow_many(hrefs, None);
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].url, "https://example.com/one");
        assert_eq!(requests[1].url, "https://example.com/base/two");
    }

    #[test]
    fn follow_urls_filters_empty_strings() {
        let request = Request::<()>::new("https://example.com/base/");
        let response = Response {
            url: "https://example.com/base/".to_string(),
            status: 200,
            headers: Headers::new(),
            body: Bytes::new(),
            request,
        };
        let hrefs = vec!["/one", "", "two"];
        let requests = response.follow_urls(hrefs);
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].url, "https://example.com/one");
        assert_eq!(requests[1].url, "https://example.com/base/two");
    }

    #[test]
    fn html_element_select_or_empty_returns_empty_on_invalid() {
        let body = b"<div class=\"parent\"><span class=\"child\">Text</span></div>";
        let response = Response {
            url: "https://example.com".to_string(),
            status: 200,
            headers: Headers::new(),
            body: Bytes::from_static(body),
            request: Request::<()>::new("https://example.com"),
        };
        let html = response.into_html(1024);
        let parent = html.select_first_or_none(".parent").unwrap();

        // Invalid selector returns empty
        assert_eq!(parent.select_or_empty("span[invalid").len(), 0);

        // Valid selector works
        let children = parent.select_or_empty(".child");
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].text(), "Text");
    }
}
