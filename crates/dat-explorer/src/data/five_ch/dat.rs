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

mod read_cgi_mapper;

use crate::data::five_ch::dat::read_cgi_mapper::html_to_dat;
use crate::data::five_ch::decode_cp932_lossy;
use crate::model::DownloadedDat;
use regex::Regex;
use reqwest::Client;
use rust_myscript::prelude::*;
use std::sync::LazyLock;

static RE_DAT_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https://([^./]+)\.5ch\.io/([^/]+)/dat/(\d+)\.dat$").unwrap());

static RE_READ_CGI_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https://([^./]+)\.5ch\.io/test/read\.cgi/([^/]+)/(\d+)/?$").unwrap()
});

struct ParsedUrl {
    server: String,
    board: String,
    thread_id: String,
}

fn parse_url(url: &str) -> Fallible<ParsedUrl> {
    if let Some(caps) = RE_DAT_URL.captures(url) {
        return Ok(ParsedUrl {
            server: caps[1].to_string(),
            board: caps[2].to_string(),
            thread_id: caps[3].to_string(),
        });
    }
    if let Some(caps) = RE_READ_CGI_URL.captures(url) {
        return Ok(ParsedUrl {
            server: caps[1].to_string(),
            board: caps[2].to_string(),
            thread_id: caps[3].to_string(),
        });
    }
    bail!("URLの形式が不正です（dat URL または read.cgi URL を指定してください）: {url}")
}

fn build_client() -> Fallible<Client> {
    Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15")
        .build()
        .context("HTTP クライアントの初期化に失敗しました")
}

pub async fn download_dat(url: &str) -> Fallible<DownloadedDat> {
    let url_kind = parse_url(url)?;
    let client = build_client()?;

    let ParsedUrl {
        server,
        board,
        thread_id,
    } = url_kind;

    let dat_url = format!("https://{server}.5ch.io/{board}/dat/{thread_id}.dat");
    let read_cgi_url = format!("https://{server}.5ch.io/test/read.cgi/{board}/{thread_id}/");
    let text = download_from_urls(&client, &dat_url, &read_cgi_url).await?;
    Ok(DownloadedDat::from_text(text))
}

async fn download_from_urls(
    client: &Client,
    dat_url: &str,
    read_cgi_url: &str,
) -> Fallible<String> {
    // Try direct dat download first
    let dat_resp = client
        .get(dat_url)
        .send()
        .await
        .with_context(|| format!("dat の取得に失敗しました: {dat_url}"))?;
    let dat_text = if dat_resp.status().is_success() {
        info!(url = %dat_url, "fetched dat directly");
        let bytes = dat_resp
            .bytes()
            .await
            .with_context(|| format!("dat のレスポンス読み取りに失敗しました: {dat_url}"))?;
        decode_cp932_lossy(&bytes)
    } else if dat_resp.status() == reqwest::StatusCode::NOT_FOUND {
        // dat not found (dat落ち) — fall back to read.cgi
        info!(url = %read_cgi_url, "dat not found, falling back to read.cgi");
        let html_resp = client
            .get(read_cgi_url)
            .send()
            .await
            .with_context(|| format!("read.cgi の取得に失敗しました: {read_cgi_url}"))?;
        if !html_resp.status().is_success() {
            bail!(
                "read.cgi取得失敗: {} ({})",
                read_cgi_url,
                html_resp.status()
            );
        }
        let bytes = html_resp.bytes().await.with_context(|| {
            format!("read.cgi のレスポンス読み取りに失敗しました: {read_cgi_url}")
        })?;
        let html = decode_cp932_lossy(&bytes);
        html_to_dat(&html)
    } else {
        bail!("dat取得失敗: {} ({})", dat_url, dat_resp.status());
    };

    Ok(dat_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn spawn_server(
        dat_status: axum::http::StatusCode,
        dat: &str,
        html_status: axum::http::StatusCode,
        html: &str,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let dat = encoding_rs::SHIFT_JIS.encode(dat).0.into_owned();
        let html = encoding_rs::SHIFT_JIS.encode(html).0.into_owned();
        let app = axum::Router::new()
            .route(
                "/thread.dat",
                axum::routing::get(move || async move { (dat_status, dat) }),
            )
            .route(
                "/read.cgi",
                axum::routing::get(move || async move { (html_status, html) }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{address}"), task)
    }

    #[tokio::test]
    async fn download_direct_dat_decodes_cp932_without_using_html() {
        use axum::http::StatusCode;
        let dat = "名無し<><>2026/01/01 ID:a<>本文<>タイトル";
        let (base, task) =
            spawn_server(StatusCode::OK, dat, StatusCode::INTERNAL_SERVER_ERROR, "").await;
        let result = download_from_urls(
            &build_client().unwrap(),
            &format!("{base}/thread.dat"),
            &format!("{base}/read.cgi"),
        )
        .await;
        task.abort();
        assert_eq!(result.unwrap(), dat);
    }

    #[tokio::test]
    async fn download_404_falls_back_to_html() {
        use axum::http::StatusCode;
        let html = r#"<h1>タイトル</h1><div class="clear post"><span class="postid">1</span><span class="postusername">名無し</span><span class="date">2026/01/01</span><div class="post-content">本文</div></div>"#;
        let (base, task) = spawn_server(StatusCode::NOT_FOUND, "", StatusCode::OK, html).await;
        let result = download_from_urls(
            &build_client().unwrap(),
            &format!("{base}/thread.dat"),
            &format!("{base}/read.cgi"),
        )
        .await;
        task.abort();
        assert_eq!(result.unwrap(), "名無し<><>2026/01/01<>本文<>タイトル");
    }

    #[tokio::test]
    async fn download_http_errors() {
        use axum::http::StatusCode;
        for (dat_status, html_status, expected) in [
            (StatusCode::FORBIDDEN, StatusCode::OK, "dat取得失敗"),
            (
                StatusCode::NOT_FOUND,
                StatusCode::INTERNAL_SERVER_ERROR,
                "read.cgi取得失敗",
            ),
        ] {
            let (base, task) = spawn_server(dat_status, "", html_status, "").await;
            let result = download_from_urls(
                &build_client().unwrap(),
                &format!("{base}/thread.dat"),
                &format!("{base}/read.cgi"),
            )
            .await;
            task.abort();
            assert!(result.unwrap_err().to_string().contains(expected));
        }
    }

    #[test]
    fn parse_url_dat() {
        let parsed = parse_url("https://server.5ch.io/board/dat/1234567890.dat").unwrap();
        assert_eq!(parsed.server, "server");
        assert_eq!(parsed.board, "board");
        assert_eq!(parsed.thread_id, "1234567890");
    }

    #[test]
    fn parse_url_read_cgi_with_slash() {
        let parsed = parse_url("https://server.5ch.io/test/read.cgi/board/1234567890/").unwrap();
        assert_eq!(parsed.server, "server");
        assert_eq!(parsed.board, "board");
        assert_eq!(parsed.thread_id, "1234567890");
    }

    #[test]
    fn parse_url_read_cgi_without_slash() {
        let parsed = parse_url("https://server.5ch.io/test/read.cgi/board/1234567890").unwrap();
        assert_eq!(parsed.server, "server");
        assert_eq!(parsed.board, "board");
        assert_eq!(parsed.thread_id, "1234567890");
    }

    #[test]
    fn parse_url_invalid() {
        assert!(parse_url("https://example.com/foo.dat").is_err());
        assert!(parse_url("https://server.5ch.io/board/1775289664").is_err());
    }
}
