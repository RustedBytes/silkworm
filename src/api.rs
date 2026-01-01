use crate::errors::{SilkwormError, SilkwormResult};

pub async fn fetch_html(url: &str) -> SilkwormResult<(String, scraper::Html)> {
    let client = wreq::Client::builder()
        .redirect(wreq::redirect::Policy::none())
        .build()
        .map_err(|err| SilkwormError::Http(err.to_string()))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|err| SilkwormError::Http(err.to_string()))?;
    let text = response
        .text()
        .await
        .map_err(|err| SilkwormError::Http(err.to_string()))?;
    let document = scraper::Html::parse_document(&text);
    Ok((text, document))
}
