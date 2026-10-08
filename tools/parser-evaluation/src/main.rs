#![forbid(unsafe_code)]

use silkworm_parser_evaluation::{fixture, scraper_count, tl_count};

fn main() {
    let html = fixture(3);
    println!("selector\tscraper\trustedbytes-tl");
    for selector in [
        "li",
        "a",
        ".item",
        "li > a",
        "li a",
        "a[href]",
        ".item a",
        "li:first-child",
        "li:nth-child(2)",
        "li:not(.missing)",
        "li + li",
        "li ~ li",
        "a[href^='/items/']",
        "li:has(a)",
    ] {
        println!(
            "{selector}\t{:?}\t{:?}",
            scraper_count(&html, selector),
            tl_count(&html, selector)
        );
    }
    let table = "<table><tr><td>Cell</td></tr></table>";
    println!(
        "implicit tbody\t{:?}\t{:?}",
        scraper_count(table, "table > tbody > tr"),
        tl_count(table, "table > tbody > tr")
    );
    let entity = "<p>A &amp; B&nbsp;C</p>";
    let doc = scraper::Html::parse_document(entity);
    let p = scraper::Selector::parse("p").unwrap();
    let text = doc.select(&p).next().unwrap().text().collect::<String>();
    let dom = tl::parse(entity, tl::ParserOptions::default()).unwrap();
    let node = dom
        .query_selector("p")
        .unwrap()
        .next()
        .unwrap()
        .get(dom.parser())
        .unwrap();
    println!("entity text\t{text:?}\t{:?}", node.inner_text(dom.parser()));
}
