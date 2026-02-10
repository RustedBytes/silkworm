use serde::Serialize;

use silkworm::{crawl_with, prelude::*};

struct QuotesSpider;

#[derive(Debug, Serialize)]
struct QuoteOnly {
    quote: String,
}

#[derive(Debug, Serialize)]
struct QuoteItem {
    text: String,
    author: String,
}

impl Spider for QuotesSpider {
    fn name(&self) -> &str {
        "quotes_xpath"
    }

    fn start_urls(&self) -> Vec<&str> {
        vec!["https://quotes.toscrape.com/page/1/"]
    }

    async fn parse(&self, response: HtmlResponse<Self>) -> SpiderResult<Self> {
        let mut out = Vec::new();

        for node in response.xpath_or_empty("//span[@class='text']") {
            let quote = QuoteOnly { quote: node.text() };
            if let Ok(item) = item_from(quote) {
                out.push(item.into());
            }
        }

        for quote in response.select_or_empty(".quote") {
            let text = quote.text_from(".text");
            let author = quote.text_from(".author");
            if text.is_empty() || author.is_empty() {
                continue;
            }

            let quote = QuoteItem { text, author };
            if let Ok(item) = item_from(quote) {
                out.push(item.into());
            }
        }

        Ok(out)
    }
}

#[tokio::main]
async fn main() -> silkworm::SilkwormResult<()> {
    let config = RunConfig::new().with_fail_fast(true);
    crawl_with(QuotesSpider, config).await
}
