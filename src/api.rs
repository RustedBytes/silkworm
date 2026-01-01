use std::sync::OnceLock;

use crate::errors::{SilkwormError, SilkwormResult};

pub async fn fetch_html(url: &str) -> SilkwormResult<(String, scraper::Html)> {
    static CLIENT: OnceLock<wreq::Client> = OnceLock::new();
    let client = if let Some(client) = CLIENT.get() {
        client.clone()
    } else {
        let built = wreq::Client::builder()
            .redirect(wreq::redirect::Policy::none())
            .build()
            .map_err(|err| SilkwormError::Http(err.to_string()))?;
        let _ = CLIENT.set(built.clone());
        built
    };
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

#[cfg(test)]
mod tests {
    use super::fetch_html;
    use crate::errors::SilkwormError;
    use scraper::Selector;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn start_test_server(body: &str) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let body = body.to_string();
        let handle = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let mut request = Vec::new();
                loop {
                    let read = match socket.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(read) => read,
                    };
                    request.extend_from_slice(&buf[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        (format!("http://{}", addr), handle)
    }

    #[tokio::test]
    async fn fetch_html_returns_text_and_document() {
        let body = "<html><body><h1>Hello</h1></body></html>";
        let (url, handle) = start_test_server(body).await;

        let (text, document) = fetch_html(&url).await.expect("fetch html");

        assert_eq!(text, body);
        let selector = Selector::parse("h1").expect("selector");
        let heading = document.select(&selector).next().expect("heading");
        assert_eq!(heading.text().collect::<String>(), "Hello");
        handle.await.expect("server task");
    }

    #[tokio::test]
    async fn fetch_html_invalid_url_returns_error() {
        let result = fetch_html("http://[::1").await;

        match result {
            Err(SilkwormError::Http(_)) => {}
            Ok(_) => panic!("expected error, got ok"),
            Err(other) => panic!("expected http error, got {other:?}"),
        }
    }
}
