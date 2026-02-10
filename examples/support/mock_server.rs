use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub const OFFLINE_ENV: &str = "SILKWORM_EXAMPLE_OFFLINE";

pub fn offline_mode_enabled() -> bool {
    std::env::var(OFFLINE_ENV)
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

pub async fn maybe_start() -> std::io::Result<Option<MockServer>> {
    if !offline_mode_enabled() {
        return Ok(None);
    }
    Ok(Some(MockServer::start().await?))
}

pub struct MockServer {
    base_url: String,
    handle: tokio::task::JoinHandle<()>,
}

#[allow(dead_code)]
impl MockServer {
    async fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let base_url = format!("http://{address}");
        let server_base_url = base_url.clone();
        let handle = tokio::spawn(async move {
            run_server(listener, server_base_url).await;
        });
        Ok(Self { base_url, handle })
    }

    pub fn quotes_root_url(&self) -> String {
        format!("{}/quotes/", self.base_url)
    }

    pub fn quotes_page_url(&self, page: usize) -> String {
        format!("{}/quotes/page/{page}/", self.base_url)
    }

    pub fn hackernews_newest_url(&self) -> String {
        format!("{}/hackernews/newest", self.base_url)
    }

    pub fn lobsters_url(&self) -> String {
        format!("{}/lobsters/", self.base_url)
    }

    pub fn sitemap_url(&self) -> String {
        format!("{}/sitemap.xml", self.base_url)
    }

    pub fn metadata_page_url(&self, page: usize) -> String {
        format!("{}/metadata/page-{page}", self.base_url)
    }

    pub fn write_url_titles_file(&self) -> std::io::Result<PathBuf> {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "silkworm_example_urls_{}_{}.jl",
            std::process::id(),
            suffix
        ));
        let content = format!(
            "{{\"url\":\"{}\"}}\n{{\"url\":\"{}\"}}\n",
            self.metadata_page_url(1),
            self.metadata_page_url(2)
        );
        fs::write(&path, content)?;
        Ok(path)
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn run_server(listener: TcpListener, base_url: String) {
    while let Ok((socket, _)) = listener.accept().await {
        let route_base_url = base_url.clone();
        tokio::spawn(async move {
            let _ = handle_connection(socket, &route_base_url).await;
        });
    }
}

async fn handle_connection(mut socket: TcpStream, base_url: &str) -> std::io::Result<()> {
    let mut buffer = [0u8; 2048];
    let mut request = Vec::new();
    loop {
        let read = socket.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if request.len() >= 64 * 1024 {
            break;
        }
    }

    let request_text = String::from_utf8_lossy(&request);
    let request_line = request_text.lines().next().unwrap_or("");
    let target = request_line.split_whitespace().nth(1).unwrap_or("/");
    let (status, content_type, body) = route(target, base_url);
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        status,
        reason_phrase(status),
        body.len()
    );
    socket.write_all(response.as_bytes()).await?;
    Ok(())
}

fn route(target: &str, base_url: &str) -> (u16, &'static str, String) {
    match target {
        "/quotes/" | "/quotes/page/1/" => (200, "text/html; charset=utf-8", quotes_page_one()),
        "/quotes/page/2/" => (200, "text/html; charset=utf-8", quotes_page_two()),
        "/hackernews/newest" => (200, "text/html; charset=utf-8", hackernews_page_one()),
        "/hackernews/newest?p=2" => (200, "text/html; charset=utf-8", hackernews_page_two()),
        "/lobsters/" => (200, "text/html; charset=utf-8", lobsters_page_one()),
        "/lobsters/page/2" => (200, "text/html; charset=utf-8", lobsters_page_two()),
        "/sitemap.xml" => (200, "application/xml; charset=utf-8", sitemap_xml(base_url)),
        "/metadata/page-1" => (
            200,
            "text/html; charset=utf-8",
            metadata_page(
                "Example Article One",
                "Short summary for article one.",
                "example,one",
                "alice",
                "og-one.png",
            ),
        ),
        "/metadata/page-2" => (
            200,
            "text/html; charset=utf-8",
            metadata_page(
                "Example Article Two",
                "Short summary for article two.",
                "example,two",
                "bob",
                "og-two.png",
            ),
        ),
        _ => (404, "text/plain; charset=utf-8", "not found".to_string()),
    }
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        404 => "Not Found",
        _ => "OK",
    }
}

fn quotes_page_one() -> String {
    r#"<!doctype html>
<html>
  <body>
    <div class="quote">
      <span class="text">"Offline quote one"</span>
      <small class="author">Author One</small>
      <div class="tags"><a class="tag">offline</a><a class="tag">demo</a></div>
    </div>
    <div class="quote">
      <span class="text">"Offline quote two"</span>
      <small class="author">Author Two</small>
      <div class="tags"><a class="tag">rust</a><a class="tag">crawler</a></div>
    </div>
    <li class="next"><a href="/quotes/page/2/">Next</a></li>
  </body>
</html>
"#
    .to_string()
}

fn quotes_page_two() -> String {
    r#"<!doctype html>
<html>
  <body>
    <div class="quote">
      <span class="text">"Offline quote page two"</span>
      <small class="author">Author Three</small>
      <div class="tags"><a class="tag">page2</a></div>
    </div>
  </body>
</html>
"#
    .to_string()
}

fn hackernews_page_one() -> String {
    r#"<!doctype html>
<html>
  <body>
    <table>
      <tr class="athing" id="1001">
        <td class="rank">1.</td>
        <td><span class="titleline"><a href="https://example.com/story-one">Story One</a></span></td>
      </tr>
      <tr>
        <td colspan="2">
          <span class="subtext">
            <span class="score">42 points</span>
            by <a class="hnuser" href="user?id=alice">alice</a>
            <span class="age"><a href="item?id=1001">1 hour ago</a></span>
            <a href="item?id=1001">17 comments</a>
          </span>
        </td>
      </tr>
      <tr class="athing" id="1002">
        <td class="rank">2.</td>
        <td><span class="titleline"><a href="/metadata/page-1">Story Two</a></span></td>
      </tr>
      <tr>
        <td colspan="2">
          <span class="subtext">
            <span class="score">7 points</span>
            by <a class="hnuser" href="user?id=bob">bob</a>
            <span class="age"><a href="item?id=1002">2 hours ago</a></span>
            <a href="item?id=1002">3 comments</a>
          </span>
        </td>
      </tr>
    </table>
    <a class="morelink" href="/hackernews/newest?p=2">More</a>
  </body>
</html>
"#
    .to_string()
}

fn hackernews_page_two() -> String {
    r#"<!doctype html>
<html>
  <body>
    <table>
      <tr class="athing" id="1003">
        <td class="rank">3.</td>
        <td><span class="titleline"><a href="/metadata/page-2">Story Three</a></span></td>
      </tr>
      <tr>
        <td colspan="2">
          <span class="subtext">
            <span class="score">1 points</span>
            by <a class="hnuser" href="user?id=carol">carol</a>
            <span class="age"><a href="item?id=1003">3 hours ago</a></span>
            <a href="item?id=1003">1 comment</a>
          </span>
        </td>
      </tr>
    </table>
  </body>
</html>
"#
    .to_string()
}

fn lobsters_page_one() -> String {
    r#"<!doctype html>
<html>
  <body>
    <ol class="stories">
      <li class="story" data-shortid="abc12">
        <span class="link"><a class="u-url" href="/metadata/page-1">Local story one</a></span>
        <a class="domain" href="https://example.com">example.com</a>
        <span class="tags"><a class="tag" href="/t/rust">rust</a><a class="tag" href="/t/web">web</a></span>
        <div class="byline">
          <a class="u-author" href="/~alice">alice</a>
          <time>1 hour ago</time>
        </div>
        <div class="voters"><span class="upvoter">12</span></div>
        <div class="comments_label"><a href="/s/abc12/local_story_one">5 comments</a></div>
      </li>
      <li class="story" id="story_def34">
        <span class="link"><a class="u-url" href="https://example.com/remote">Local story two</a></span>
        <a class="domain" href="https://example.com">example.com</a>
        <span class="tags"><a class="tag" href="/t/crawling">crawling</a></span>
        <div class="byline">
          <a class="u-author" href="/~bob">bob</a>
          <time>2 hours ago</time>
        </div>
        <div class="voters"><span class="upvoter">4</span></div>
        <div class="comments_label"><a href="/s/def34/local_story_two">2 comments</a></div>
      </li>
    </ol>
    <div class="morelink"><a href="/lobsters/page/2">More</a></div>
  </body>
</html>
"#
    .to_string()
}

fn lobsters_page_two() -> String {
    r#"<!doctype html>
<html>
  <body>
    <ol class="stories">
      <li class="story" data-shortid="ghi56">
        <span class="link"><a class="u-url" href="/metadata/page-2">Local story three</a></span>
        <a class="domain" href="https://example.com">example.com</a>
        <span class="tags"><a class="tag" href="/t/mock">mock</a></span>
        <div class="byline">
          <a class="u-author" href="/~carol">carol</a>
          <time>3 hours ago</time>
        </div>
        <div class="voters"><span class="upvoter">1</span></div>
        <div class="comments_label"><a href="/s/ghi56/local_story_three">1 comment</a></div>
      </li>
    </ol>
  </body>
</html>
"#
    .to_string()
}

fn sitemap_xml(base_url: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url><loc>{base_url}/metadata/page-1</loc></url>
  <url><loc>{base_url}/metadata/page-2</loc></url>
</urlset>
"#
    )
}

fn metadata_page(
    title: &str,
    description: &str,
    keywords: &str,
    author: &str,
    image_name: &str,
) -> String {
    format!(
        r#"<!doctype html>
<html>
  <head>
    <title>{title}</title>
    <link rel="canonical" href="https://example.local/{author}/{title}" />
    <meta name="description" content="{description}" />
    <meta name="keywords" content="{keywords}" />
    <meta name="author" content="{author}" />
    <meta name="robots" content="index,follow" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta property="og:title" content="{title}" />
    <meta property="og:description" content="{description}" />
    <meta property="og:type" content="article" />
    <meta property="og:url" content="https://example.local/{author}" />
    <meta property="og:image" content="https://example.local/{image_name}" />
    <meta property="og:site_name" content="Silkworm Mock" />
    <meta property="og:locale" content="en_US" />
    <meta name="twitter:card" content="summary_large_image" />
    <meta name="twitter:title" content="{title}" />
    <meta name="twitter:description" content="{description}" />
    <meta name="twitter:image" content="https://example.local/{image_name}" />
    <meta name="twitter:site" content="@silkworm_mock" />
  </head>
  <body>
    <h1>{title}</h1>
    <p>{description}</p>
  </body>
</html>
"#
    )
}
