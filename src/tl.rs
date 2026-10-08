//! Explicit TL integration (`tl-parser` feature).
//!
//! TL is a lightweight parser, not an HTML5 tree builder. Its CSS subset, raw
//! attributes and serialization differ from scraper. Existing scraper APIs are
//! unaffected; see `docs/parser-evaluation.md` for the supported integration scope.

use std::borrow::Cow;
use std::fmt;

use crate::{SilkwormError, SilkwormResult};

/// An owned, reusable DOM backed by rustedbytes-tl.
///
/// Nodes and iterators borrow this document; the input is freed with it.
///
/// ```
/// # #[cfg(feature = "tl-parser")] {
/// let doc = silkworm::TlDocument::parse("<p>A &amp; B</p>".into()).unwrap();
/// let element = doc.select_first("p").unwrap().unwrap();
/// assert_eq!(element.text(), "A & B");
/// # }
/// ```
///
/// ```compile_fail
/// # #[cfg(feature = "tl-parser")] {
/// let element = {
///     let doc = silkworm::TlDocument::parse("<p>text</p>".into()).unwrap();
///     doc.select_first("p").unwrap().unwrap()
/// };
/// println!("{}", element.text());
/// # }
/// # #[cfg(not(feature = "tl-parser"))] compile_error!("feature disabled");
/// ```
pub struct TlDocument {
    owner: ::tl::VDomGuard,
}

impl fmt::Debug for TlDocument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlDocument")
            .field("nodes", &self.owner.get_ref().nodes().len())
            .finish_non_exhaustive()
    }
}

impl TlDocument {
    /// Take ownership of HTML and parse it once through TL's safe constructor.
    ///
    /// # Errors
    /// Returns a selector error when parsing exceeds TL's input limits.
    pub fn parse(html: String) -> SilkwormResult<Self> {
        ::tl::VDomGuard::parse(html, ::tl::ParserOptions::default())
            .map(|owner| Self { owner })
            .map_err(|error| SilkwormError::Selector(format!("TL HTML parse failed: {error}")))
    }

    /// Iterate matching elements in document order without materializing a vector.
    /// The query is parsed on each call; structural queries may allocate indexes.
    ///
    /// # Errors
    /// Invalid, unsupported or overlarge selectors return an error. A valid query
    /// with no matches returns an empty iterator.
    pub fn select<'a>(
        &'a self,
        selector: &'a str,
    ) -> SilkwormResult<impl Iterator<Item = TlElement<'a>> + 'a> {
        let dom = self.owner.get_ref();
        let iter = dom
            .query_selector(selector)
            .ok_or_else(|| selector_error(selector))?;
        Ok(iter.filter_map(move |handle| {
            handle.get(dom.parser()).map(|node| TlElement {
                document: self,
                node,
            })
        }))
    }

    /// Return the first match, preserving selector errors.
    ///
    /// # Errors
    /// See [`Self::select`].
    pub fn select_first<'a>(&'a self, selector: &'a str) -> SilkwormResult<Option<TlElement<'a>>> {
        Ok(self.select(selector)?.next())
    }
}

/// An element borrowing its original TL DOM. Nested selection does not reparse HTML.
#[derive(Clone, Copy)]
pub struct TlElement<'a> {
    document: &'a TlDocument,
    node: &'a ::tl::Node<'a>,
}

impl TlElement<'_> {
    /// Concatenate decoded descendant text. Comments are ignored; no layout spaces
    /// are inserted. Script/style tags keep their raw text. The result may allocate.
    pub fn text(&self) -> Cow<'_, str> {
        self.node
            .decoded_inner_text(self.document.owner.get_ref().parser())
    }

    /// Read an attribute without entity decoding. Missing attributes return `None`;
    /// present valueless attributes return an empty string. Initial bytes borrow input.
    pub fn raw_attr<'a>(&'a self, name: &'a str) -> Option<Cow<'a, str>> {
        self.node
            .as_tag()?
            .attributes()
            .get(name)
            .map(|value| value.map_or(Cow::Borrowed(""), |bytes| bytes.as_utf8_str()))
    }

    /// Serialize with TL's rules, which do not normalize HTML like scraper.
    pub fn html(&self) -> Cow<'_, str> {
        self.node.outer_html(self.document.owner.get_ref().parser())
    }

    /// Query descendants in the original DOM, excluding this element itself.
    ///
    /// # Errors
    /// Invalid, unsupported or overlarge selectors return an error.
    pub fn select<'a>(
        &'a self,
        selector: &'a str,
    ) -> SilkwormResult<impl Iterator<Item = TlElement<'a>> + 'a> {
        let dom = self.document.owner.get_ref();
        let iter = self
            .node
            .as_tag()
            .and_then(|tag| tag.query_selector(dom.parser(), selector))
            .ok_or_else(|| selector_error(selector))?;
        Ok(iter.filter_map(move |handle| {
            handle.get(dom.parser()).map(|node| TlElement {
                document: self.document,
                node,
            })
        }))
    }
}

fn selector_error(selector: &str) -> SilkwormError {
    SilkwormError::Selector(format!(
        "Invalid, unsupported or overlarge TL CSS selector: {selector}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Headers, Request, Response};

    #[test]
    fn extraction_and_nested_queries_use_the_owned_dom() {
        let doc = TlDocument::parse(
            "<ul><li><a href='/x?a=1&amp;b=2'>A &amp; B</a></li><li disabled>Two</li></ul>".into(),
        )
        .unwrap();
        let ul = doc.select_first("ul").unwrap().unwrap();
        assert_eq!(ul.select("li").unwrap().count(), 2);
        assert_eq!(ul.select("ul").unwrap().count(), 0);
        let a = ul.select("li > a").unwrap().next().unwrap();
        assert_eq!(a.text(), "A & B");
        assert_eq!(a.raw_attr("href").unwrap(), "/x?a=1&amp;b=2");
        assert!(a.raw_attr("missing").is_none());
        assert!(a.html().contains("A &amp; B"));
        assert_eq!(
            doc.select_first("li[disabled]")
                .unwrap()
                .unwrap()
                .raw_attr("disabled")
                .unwrap(),
            ""
        );
    }

    #[test]
    fn errors_are_distinct_from_empty_results() {
        let doc = TlDocument::parse("<p>Hello</p>".into()).unwrap();
        assert!(doc.select("p:unsupported").is_err());
        assert!(doc.select("p:has(> a)").is_err());
        assert_eq!(doc.select("article").unwrap().count(), 0);
        assert!(doc.select_first("article").unwrap().is_none());
        assert!(doc.select_first("p").unwrap().unwrap().select("[").is_err());
    }

    #[test]
    fn response_snapshot_respects_decoding_size_and_mutation() {
        let mut headers = Headers::new();
        headers.insert(
            "content-type".into(),
            "text/html; charset=windows-1252".into(),
        );
        let source = Response::<()> {
            url: "https://example.test".into(),
            status: 200,
            headers,
            body: bytes::Bytes::from_static(b"<p>caf\xe9</p><p>ignored</p>"),
            request: Request::get("https://example.test"),
        };
        assert_eq!(
            source
                .clone()
                .into_html(0)
                .tl_document()
                .unwrap()
                .select("*")
                .unwrap()
                .count(),
            0
        );
        let mut response = source.into_html(11);
        let snapshot = response.tl_document().unwrap();
        assert_eq!(snapshot.select("p").unwrap().count(), 1);
        assert_eq!(
            snapshot.select_first("p").unwrap().unwrap().text(),
            "caf\u{e9}"
        );
        response.body = bytes::Bytes::from_static(b"<p>new</p>");
        assert_eq!(
            response
                .tl_document()
                .unwrap()
                .select_first("p")
                .unwrap()
                .unwrap()
                .text(),
            "new"
        );
        drop(response);
        assert_eq!(
            snapshot.select_first("p").unwrap().unwrap().text(),
            "caf\u{e9}"
        );
    }

    #[test]
    fn document_is_send_sync_and_can_move_to_a_worker() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<TlDocument>();
        let doc = TlDocument::parse("<p>worker</p>".into()).unwrap();
        let text =
            std::thread::spawn(move || doc.select_first("p").unwrap().unwrap().text().into_owned())
                .join()
                .unwrap();
        assert_eq!(text, "worker");
    }

    #[test]
    fn html5_recovery_is_explicitly_not_provided() {
        let doc = TlDocument::parse("<table><tr><td>Cell</td></tr></table>".into()).unwrap();
        assert_eq!(doc.select("table > tbody > tr").unwrap().count(), 0);
        assert_eq!(doc.select("table > tr").unwrap().count(), 1);
    }
}
