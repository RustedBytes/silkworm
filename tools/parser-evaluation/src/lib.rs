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
    fn current_tl_combinators_do_not_match_scraper() {
        let html = fixture(3);
        for selector in ["li a", "li > a", ".item a"] {
            assert_eq!(scraper_count(&html, selector), Some(3));
            assert_eq!(tl_count(&html, selector), Some(0));
        }
        assert_eq!(tl_count(&html, "li:nth-child(2)"), None);
        assert_eq!(scraper_count(&html, "li:nth-child(2)"), Some(1));
    }

    #[test]
    fn implied_table_structure_is_a_migration_blocker() {
        let html = "<table><tr><td>Cell</td></tr></table>";
        assert_eq!(scraper_count(html, "table > tbody > tr"), Some(1));
        assert_ne!(tl_count(html, "table > tbody > tr"), Some(1));
    }
}
