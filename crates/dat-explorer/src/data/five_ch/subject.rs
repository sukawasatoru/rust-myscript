/*
 * Copyright 2026 sukawasatoru
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Client for 5ch board listings (`subject.txt`).

mod subject_mapper;

use crate::data::five_ch::subject::subject_mapper::{decode_cp932, parse_subject};
use crate::model::fetch::{FetchSubjectParams, FetchSubjectResult};
use crate::model::query::filter_threads;
use reqwest::{Client, Response, Url};
use rust_myscript::prelude::*;
use std::time::Duration;

/// Fixed User-Agent. Kept constant so that the MCP client can allow-list this tool once.
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15";

/// Request timeout for a single `subject.txt` download.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum accepted body size (1 MiB). Exactly this size succeeds; one byte more fails.
const MAX_BODY_BYTES: u64 = 1024 * 1024;

/// Builds the HTTP client used to download `subject.txt`.
///
/// Public because both the binary crate and the HTTP tests need the exact same configuration.
pub fn build_client() -> Fallible<Client> {
    build_client_with_timeout(DEFAULT_TIMEOUT)
}

/// Single place where the client configuration lives, so that tests exercise the production
/// settings with only the timeout shortened.
fn build_client_with_timeout(timeout: Duration) -> Fallible<Client> {
    Client::builder()
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
        .context("HTTP クライアントの初期化に失敗しました")
}

pub async fn fetch_subject(
    client: &Client,
    params: &FetchSubjectParams,
) -> Fallible<FetchSubjectResult> {
    let url = validate_url(&params.url)?;
    let safe_url = redact_url(&url);

    let response = client
        .get(url)
        .send()
        .await
        .map_err(without_url)
        .with_context(|| format!("subject.txt の取得に失敗しました: {safe_url}"))?;

    let status = response.status();
    if status.is_redirection() {
        bail!(
            "subject.txt の取得に失敗しました（リダイレクトは追従しません）: {safe_url} ({status})"
        );
    }
    if !status.is_success() {
        bail!("subject.txt の取得に失敗しました: {safe_url} ({status})");
    }

    let body = read_body_limited(response, &safe_url).await?;
    let text = decode_cp932(&body)
        .with_context(|| format!("subject.txt のデコードに失敗しました: {safe_url}"))?;
    let threads = parse_subject(&text)
        .with_context(|| format!("subject.txt の解析に失敗しました: {safe_url}"))?;

    Ok(FetchSubjectResult {
        threads: filter_threads(threads, params.title_contains.as_deref()),
    })
}

/// Removes the URL a `reqwest::Error` carries, because it may contain userinfo.
fn without_url(e: reqwest::Error) -> reqwest::Error {
    e.without_url()
}

/// Parses and validates a user supplied URL, returning it with the fragment stripped.
fn validate_url(raw: &str) -> Fallible<Url> {
    let mut url = match Url::parse(raw) {
        Ok(url) => url,
        Err(e) => bail!("URL の形式が不正です: {e}"),
    };

    let scheme = url.scheme().to_owned();
    if scheme != "http" && scheme != "https" {
        bail!(
            "URL のスキームは http または https のみ指定できます: {} ({scheme})",
            redact_url(&url)
        );
    }

    match url.host_str() {
        Some(host) if !host.is_empty() => {}
        _ => bail!("URL にホストが含まれていません: {}", redact_url(&url)),
    }

    url.set_fragment(None);
    Ok(url)
}

/// Renders a URL without its userinfo so that it is safe to put in logs and error messages.
fn redact_url(url: &Url) -> String {
    if url.username().is_empty() && url.password().is_none() {
        return url.to_string();
    }

    let mut redacted = url.clone();
    if redacted.set_password(None).is_err() || redacted.set_username("").is_err() {
        return format!(
            "{}://{}",
            url.scheme(),
            url.host_str().unwrap_or("<unknown>")
        );
    }
    redacted.to_string()
}

/// Reads the response body, aborting as soon as [`MAX_BODY_BYTES`] is exceeded.
///
/// `Content-Length` is only a hint, so the accumulated chunk length is checked as well.
async fn read_body_limited(mut response: Response, safe_url: &str) -> Fallible<Vec<u8>> {
    if let Some(len) = response.content_length()
        && MAX_BODY_BYTES < len
    {
        bail!(
            "subject.txt のサイズが上限 {MAX_BODY_BYTES} バイトを超えています（Content-Length）: {safe_url} ({len} バイト)"
        );
    }

    let mut buf = Vec::new();
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(without_url)
            .with_context(|| format!("subject.txt の受信に失敗しました: {safe_url}"))?;
        let Some(chunk) = chunk else {
            break;
        };
        let total = buf.len() as u64 + chunk.len() as u64;
        if MAX_BODY_BYTES < total {
            bail!(
                "subject.txt のサイズが上限 {MAX_BODY_BYTES} バイトを超えています（受信中）: {safe_url}"
            );
        }
        buf.extend_from_slice(&chunk);
    }

    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fetch::SubjectThread;

    fn thread(thread_id: &str, title: &str, res_count: u64) -> SubjectThread {
        SubjectThread {
            thread_id: thread_id.to_owned(),
            title: title.to_owned(),
            res_count,
        }
    }

    #[test]
    fn validate_url_http_and_https() {
        assert_eq!(
            validate_url("http://server.5ch.io/board/subject.txt")
                .unwrap()
                .as_str(),
            "http://server.5ch.io/board/subject.txt"
        );
        assert_eq!(
            validate_url("https://server.5ch.io/board/subject.txt")
                .unwrap()
                .as_str(),
            "https://server.5ch.io/board/subject.txt"
        );
    }

    #[test]
    fn validate_url_allows_userinfo() {
        let actual = validate_url("https://user:secret@example.com/subject.txt").unwrap();
        assert_eq!(actual.username(), "user");
        assert_eq!(actual.password(), Some("secret"));
    }

    #[test]
    fn validate_url_keeps_query() {
        let actual = validate_url("https://example.com/subject.txt?a=1&b=2").unwrap();
        assert_eq!(actual.query(), Some("a=1&b=2"));
    }

    #[test]
    fn validate_url_strips_fragment() {
        let actual = validate_url("https://example.com/subject.txt?a=1#frag").unwrap();
        assert_eq!(actual.fragment(), None);
        assert_eq!(actual.as_str(), "https://example.com/subject.txt?a=1");
    }

    #[test]
    fn validate_url_rejects_other_scheme() {
        for url in [
            "ftp://example.com/subject.txt",
            "file:///tmp/subject.txt",
            "data:text/plain,hello",
        ] {
            assert!(validate_url(url).is_err(), "{url}");
        }
    }

    #[test]
    fn validate_url_rejects_relative_url() {
        for url in ["/board/subject.txt", "subject.txt", ""] {
            assert!(validate_url(url).is_err(), "{url}");
        }
    }

    #[test]
    fn validate_url_rejects_missing_host() {
        for url in ["http://", "https://", "http://:8080/", "https://:80"] {
            assert!(validate_url(url).is_err(), "{url}");
        }

        let actual = validate_url("https:///subject.txt").unwrap();
        assert_eq!(actual.host_str(), Some("subject.txt"));
    }

    #[test]
    fn validate_url_error_does_not_leak_password() {
        let actual = format!(
            "{:#}",
            validate_url("ftp://user:secret@example.com/subject.txt").unwrap_err()
        );
        assert!(!actual.contains("secret"), "{actual}");

        let actual = format!(
            "{:#}",
            validate_url("httpx://user:secret@example.com/subject.txt").unwrap_err()
        );
        assert!(!actual.contains("secret"), "{actual}");
    }

    #[test]
    fn redact_url_removes_userinfo() {
        let url = Url::parse("https://user:secret@example.com/subject.txt?a=1").unwrap();
        assert_eq!(redact_url(&url), "https://example.com/subject.txt?a=1");

        let url = Url::parse("https://user@example.com/subject.txt").unwrap();
        assert_eq!(redact_url(&url), "https://example.com/subject.txt");

        let url = Url::parse("https://example.com/subject.txt").unwrap();
        assert_eq!(redact_url(&url), "https://example.com/subject.txt");
    }

    use axum::Router;
    use axum::body::Body;
    use axum::http::{HeaderMap, StatusCode, header};
    use axum::routing::get;
    use std::sync::{Arc, Mutex};

    /// Valid `subject.txt` used by the HTTP tests, in CP932.
    fn sample_body() -> Vec<u8> {
        let (bytes, _, _) = encoding_rs::SHIFT_JIS.encode(
            "1234567890.dat<>テストスレッド★630  (649)\n\
             1234567892.dat<>🥺絵文字テストスレッド🥺★631  (398)\n\
             1234567893.dat<>Test Thread Alpha  (427)\n",
        );
        bytes.into_owned()
    }

    async fn spawn_server(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        format!("http://{addr}")
    }

    async fn spawn_bytes_server(status: StatusCode, body: Vec<u8>) -> String {
        let body = Arc::new(body);
        let router = Router::new().route(
            "/subject.txt",
            get(move || {
                let body = body.clone();
                async move {
                    (
                        status,
                        [(header::CONTENT_TYPE, "text/plain")],
                        body.as_ref().clone(),
                    )
                }
            }),
        );
        spawn_server(router).await
    }

    async fn fetch(base_url: &str, title_contains: Option<&str>) -> Fallible<FetchSubjectResult> {
        let client = build_client()?;
        fetch_subject(
            &client,
            &FetchSubjectParams {
                url: format!("{base_url}/subject.txt"),
                title_contains: title_contains.map(str::to_owned),
            },
        )
        .await
    }

    #[tokio::test]
    async fn http_ok() {
        let base_url = spawn_bytes_server(StatusCode::OK, sample_body()).await;
        let actual = fetch(&base_url, None).await.unwrap();
        assert_eq!(
            actual.threads,
            vec![
                thread("1234567890", "テストスレッド★630", 649),
                thread("1234567892", "🥺絵文字テストスレッド🥺★631", 398),
                thread("1234567893", "Test Thread Alpha", 427),
            ]
        );
    }

    #[tokio::test]
    async fn http_sends_fixed_user_agent() {
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let router = {
            let seen = seen.clone();
            Router::new().route(
                "/subject.txt",
                get(move |headers: HeaderMap| {
                    let seen = seen.clone();
                    async move {
                        let ua = headers
                            .get(header::USER_AGENT)
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or_default()
                            .to_owned();
                        seen.lock().unwrap().push(ua);
                        (StatusCode::OK, sample_body())
                    }
                }),
            )
        };
        let base_url = spawn_server(router).await;

        fetch(&base_url, None).await.unwrap();
        assert_eq!(seen.lock().unwrap().as_slice(), [USER_AGENT.to_owned()]);
    }

    #[tokio::test]
    async fn http_does_not_follow_redirect() {
        let router = Router::new()
            .route(
                "/subject.txt",
                get(|| async {
                    (
                        StatusCode::FOUND,
                        [(header::LOCATION, "/moved.txt")],
                        Vec::<u8>::new(),
                    )
                }),
            )
            .route(
                "/moved.txt",
                get(|| async { (StatusCode::OK, sample_body()) }),
            );
        let base_url = spawn_server(router).await;

        let actual = fetch(&base_url, None).await.unwrap_err();
        let actual = format!("{actual:#}");
        assert!(actual.contains("302"), "{actual}");
    }

    #[tokio::test]
    async fn http_client_error() {
        let base_url = spawn_bytes_server(StatusCode::NOT_FOUND, Vec::new()).await;
        let actual = format!("{:#}", fetch(&base_url, None).await.unwrap_err());
        assert!(actual.contains("404"), "{actual}");
    }

    #[tokio::test]
    async fn http_server_error() {
        let base_url = spawn_bytes_server(StatusCode::INTERNAL_SERVER_ERROR, Vec::new()).await;
        let actual = format!("{:#}", fetch(&base_url, None).await.unwrap_err());
        assert!(actual.contains("500"), "{actual}");
    }

    #[tokio::test]
    async fn http_error_does_not_leak_password() {
        let base_url = spawn_bytes_server(StatusCode::NOT_FOUND, Vec::new()).await;
        let host = base_url.trim_start_matches("http://").to_owned();
        let client = build_client().unwrap();
        let actual = fetch_subject(
            &client,
            &FetchSubjectParams {
                url: format!("http://user:secret@{host}/subject.txt"),
                title_contains: None,
            },
        )
        .await
        .unwrap_err();
        let actual = format!("{actual:#?}\n{actual:#}");
        assert!(!actual.contains("secret"), "{actual}");
    }

    #[tokio::test]
    async fn http_timeout() {
        let router = Router::new().route(
            "/subject.txt",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                (StatusCode::OK, sample_body())
            }),
        );
        let base_url = spawn_server(router).await;

        let client = build_client_with_timeout(Duration::from_millis(200)).unwrap();
        let actual = fetch_subject(
            &client,
            &FetchSubjectParams {
                url: format!("{base_url}/subject.txt"),
                title_contains: None,
            },
        )
        .await;
        assert!(actual.is_err());
    }

    /// Builds a valid `subject.txt` of exactly `size` bytes by padding the last title.
    fn sized_body(size: usize) -> Vec<u8> {
        let head = "1234567890.dat<>head  (1)\n";
        let prefix = "1234567891.dat<>";
        let suffix = "  (2)\n";
        let filler = size - head.len() - prefix.len() - suffix.len();
        let body = format!("{head}{prefix}{}{suffix}", "a".repeat(filler));
        assert_eq!(body.len(), size);
        body.into_bytes()
    }

    #[tokio::test]
    async fn http_body_exactly_max_is_ok() {
        let body = sized_body(MAX_BODY_BYTES as usize);
        let base_url = spawn_bytes_server(StatusCode::OK, body).await;
        let actual = fetch(&base_url, None).await.unwrap();
        assert_eq!(actual.threads.len(), 2);
        assert_eq!(actual.threads[0], thread("1234567890", "head", 1));
        assert_eq!(actual.threads[1].res_count, 2);
    }

    #[tokio::test]
    async fn http_body_over_max_with_content_length() {
        let body = sized_body(MAX_BODY_BYTES as usize + 1);
        let base_url = spawn_bytes_server(StatusCode::OK, body).await;
        let actual = format!("{:#}", fetch(&base_url, None).await.unwrap_err());
        assert!(actual.contains("Content-Length"), "{actual}");
    }

    #[tokio::test]
    async fn http_body_over_max_chunked() {
        let router = Router::new().route(
            "/subject.txt",
            get(|| async {
                let chunk = vec![b'a'; 64 * 1024];
                let chunks = (0..20)
                    .map(move |_| Ok::<_, std::io::Error>(chunk.clone()))
                    .collect::<Vec<_>>();
                Body::from_stream(futures::stream::iter(chunks))
            }),
        );
        let base_url = spawn_server(router).await;

        let actual = format!("{:#}", fetch(&base_url, None).await.unwrap_err());
        assert!(actual.contains("受信中"), "{actual}");
    }

    #[tokio::test]
    async fn http_body_exactly_max_chunked_is_ok() {
        let router = Router::new().route(
            "/subject.txt",
            get(|| async {
                let body = sized_body(MAX_BODY_BYTES as usize);
                let chunks = body
                    .chunks(64 * 1024)
                    .map(|c| Ok::<_, std::io::Error>(c.to_vec()))
                    .collect::<Vec<_>>();
                Body::from_stream(futures::stream::iter(chunks))
            }),
        );
        let base_url = spawn_server(router).await;

        let actual = fetch(&base_url, None).await.unwrap();
        assert_eq!(actual.threads.len(), 2);
    }

    #[tokio::test]
    async fn http_empty_body() {
        let base_url = spawn_bytes_server(StatusCode::OK, Vec::new()).await;
        let actual = fetch(&base_url, None).await.unwrap();
        assert!(actual.threads.is_empty());
    }

    #[tokio::test]
    async fn http_blank_lines_only_body() {
        for body in [b"\n\n\n".as_slice(), b"\r\n\r\n".as_slice()] {
            let base_url = spawn_bytes_server(StatusCode::OK, body.to_vec()).await;
            let actual = fetch(&base_url, None).await.unwrap();
            assert!(actual.threads.is_empty());
        }
    }

    #[tokio::test]
    async fn http_html_body_is_parse_error() {
        let body = b"<html><body>Not Found</body></html>".to_vec();
        let base_url = spawn_bytes_server(StatusCode::OK, body).await;
        let actual = format!("{:#}", fetch(&base_url, None).await.unwrap_err());
        assert!(actual.contains("解析"), "{actual}");
        assert!(actual.contains("1 行目"), "{actual}");
    }

    #[tokio::test]
    async fn http_invalid_cp932_body_is_decode_error() {
        let base_url = spawn_bytes_server(StatusCode::OK, vec![0x81, 0x20]).await;
        let actual = format!("{:#}", fetch(&base_url, None).await.unwrap_err());
        assert!(actual.contains("デコード"), "{actual}");
    }

    #[tokio::test]
    async fn http_title_contains_filter() {
        let base_url = spawn_bytes_server(StatusCode::OK, sample_body()).await;
        let actual = fetch(&base_url, Some("★630")).await.unwrap();
        assert_eq!(
            actual.threads,
            vec![thread("1234567890", "テストスレッド★630", 649)]
        );

        let actual = fetch(&base_url, Some("")).await.unwrap();
        assert_eq!(actual.threads.len(), 3);

        let actual = fetch(&base_url, Some("存在しない")).await.unwrap();
        assert!(actual.threads.is_empty());
    }

    #[tokio::test]
    async fn http_keeps_query_and_strips_fragment() {
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let router = {
            let seen = seen.clone();
            Router::new().route(
                "/subject.txt",
                get(move |uri: axum::http::Uri| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push(uri.to_string());
                        (StatusCode::OK, sample_body())
                    }
                }),
            )
        };
        let base_url = spawn_server(router).await;

        let client = build_client().unwrap();
        fetch_subject(
            &client,
            &FetchSubjectParams {
                url: format!("{base_url}/subject.txt?board=test#frag"),
                title_contains: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(
            seen.lock().unwrap().as_slice(),
            ["/subject.txt?board=test".to_owned()]
        );
    }

    #[tokio::test]
    async fn http_invalid_url_is_error() {
        let client = build_client().unwrap();
        for url in ["ftp://example.com/subject.txt", "subject.txt", "http://"] {
            let actual = fetch_subject(
                &client,
                &FetchSubjectParams {
                    url: url.to_owned(),
                    title_contains: None,
                },
            )
            .await;
            assert!(actual.is_err(), "{url}");
        }
    }
}
