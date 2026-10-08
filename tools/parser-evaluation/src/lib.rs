#![forbid(unsafe_code)]

use std::fmt::Write as _;

pub fn fixture(rows: usize) -> String {
    let mut html = String::from("<!doctype html><html><body><ul>");
    for i in 0..rows {
        write!(
            html,
            "<li class=\"item\"><a href=\"/items/{i}\">Item {i}</a></li>"
        )
        .unwrap();
    }
    html.push_str("</ul></body></html>");
    html
}

pub fn scraper_count(html: &str, selector: &str) -> Option<usize> {
    let selector = scraper::Selector::parse(selector).ok()?;
    Some(
        scraper::Html::parse_document(html)
            .select(&selector)
            .count(),
    )
}

pub fn tl_count(html: &str, selector: &str) -> Option<usize> {
    let dom = tl::parse(html, tl::ParserOptions::default()).ok()?;
    Some(dom.query_selector(selector)?.count())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_subset_agrees_on_well_formed_catalog() {
        let html = fixture(10);
        for selector in ["li", ".item", "a[href]"] {
            assert_eq!(scraper_count(&html, selector), Some(10));
            assert_eq!(tl_count(&html, selector), Some(10));
        }
    }

    #[test]
    fn published_tl_structural_selectors_match_scraper() {
        let html = fixture(3);
        for selector in ["li a", "li > a", ".item a", "li:not(.missing)", "li:has(a)"] {
            assert_eq!(scraper_count(&html, selector), Some(3));
            assert_eq!(tl_count(&html, selector), Some(3));
        }
        assert_eq!(tl_count(&html, "li:nth-child(2)"), Some(1));
        assert_eq!(scraper_count(&html, "li:nth-child(2)"), Some(1));
        for selector in [
            "li:first-child",
            "li + li",
            "li ~ li",
            "li:nth-child(odd)",
            "li:nth-child(-n+2)",
        ] {
            assert_eq!(tl_count(&html, selector), scraper_count(&html, selector));
        }
    }

    #[test]
    fn implied_table_structure_is_a_migration_blocker() {
        let html = "<table><tr><td>Cell</td></tr></table>";
        assert_eq!(scraper_count(html, "table > tbody > tr"), Some(1));
        assert_ne!(tl_count(html, "table > tbody > tr"), Some(1));
    }
    #[test]
    fn decoded_text_agrees_on_references_and_mixed_content() {
        for html in [
            "<p>A &amp; B&nbsp;C &#x1F980;</p>",
            "<p>Hello<b>world</b><!-- comment -->!</p>",
            "<p>&NotEqualTilde; &#0; &#128;</p>",
            "<script>x &amp; y</script>",
        ] {
            let selector = if html.starts_with("<script") {
                "script"
            } else {
                "p"
            };
            let query = scraper::Selector::parse(selector).unwrap();
            let doc = scraper::Html::parse_document(html);
            let expected = doc
                .select(&query)
                .next()
                .unwrap()
                .text()
                .collect::<String>();
            let dom = tl::parse(html, Default::default()).unwrap();
            let handle = dom.query_selector(selector).unwrap().next().unwrap();
            assert_eq!(
                handle
                    .get(dom.parser())
                    .unwrap()
                    .decoded_inner_text(dom.parser()),
                expected,
                "{html}"
            );
        }
    }

    #[test]
    fn attributes_and_unsupported_css_remain_different() {
        let html = "<a href='/x?a=1&amp;b=2'>link</a>";
        let doc = scraper::Html::parse_document(html);
        let query = scraper::Selector::parse("a").unwrap();
        assert_eq!(
            doc.select(&query).next().unwrap().attr("href"),
            Some("/x?a=1&b=2")
        );
        let dom = tl::parse(html, Default::default()).unwrap();
        let handle = dom.query_selector("a").unwrap().next().unwrap();
        assert_eq!(
            handle
                .get(dom.parser())
                .unwrap()
                .as_tag()
                .unwrap()
                .attributes()
                .get("href")
                .unwrap()
                .unwrap()
                .as_bytes(),
            b"/x?a=1&amp;b=2"
        );
        assert_eq!(scraper_count(html, "a:has(> *)"), Some(0));
        assert_eq!(tl_count(html, "a:has(> *)"), None);
    }
}
