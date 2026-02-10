use serde::Serialize;

use silkworm::{crawl_with, prelude::*};

#[path = "support/mock_server.rs"]
mod mock_server;

struct QuotesSpider {
    start_url: String,
}

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

    async fn start_requests(&self) -> Vec<Request<Self>> {
        vec![Request::get(self.start_url.clone())]
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
    let mock = mock_server::maybe_start().await?;
    let start_url = mock
        .as_ref()
        .map(|server| server.quotes_page_url(1))
        .unwrap_or_else(|| "https://quotes.toscrape.com/page/1/".to_string());
    let config = RunConfig::new().with_fail_fast(true);
    crawl_with(QuotesSpider { start_url }, config).await
}
