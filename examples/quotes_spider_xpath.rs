use async_trait::async_trait;
use serde_json::json;

use silkworm::{run_spider, HtmlResponse, Spider, SpiderResult};

struct QuotesSpider;

#[async_trait]
impl Spider for QuotesSpider {
    fn name(&self) -> &str {
        "quotes_xpath"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://quotes.toscrape.com/page/1/".to_string()]
    }

    async fn parse(&self, response: HtmlResponse<Self>) -> SpiderResult<Self> {
        let mut out = Vec::new();

        match response.xpath("//span[@class='text']") {
            Ok(nodes) => {
                for node in nodes {
                    out.push(json!({"quote": node.text()}).into());
                }
            }
            Err(err) => {
                self.log().warn("XPath not supported", &[("error", err.to_string())]);
            }
        }

        let quotes = match response.select(".quote") {
            Ok(nodes) => nodes,
            Err(_) => return out,
        };

        for quote in quotes {
            let text_el = match quote.select_first(".text") {
                Ok(Some(el)) => el,
                _ => continue,
            };
            let author_el = match quote.select_first(".author") {
                Ok(Some(el)) => el,
                _ => continue,
            };

            out.push(
                json!({
                    "text": text_el.text(),
                    "author": author_el.text(),
                })
                .into(),
            );
        }

        out
    }
}

fn main() -> silkworm::SilkwormResult<()> {
    run_spider(QuotesSpider)
}
