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

use regex::Regex;
use reqwest::Client;
use rust_myscript::prelude::*;
use std::path::Path;
use std::sync::LazyLock;

static RE_DAT_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https://([^./]+)\.5ch\.io/([^/]+)/dat/(\d+)\.dat$").unwrap());

static RE_READ_CGI_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https://([^./]+)\.5ch\.io/test/read\.cgi/([^/]+)/(\d+)/?$").unwrap()
});

static RE_POST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"class="clear post""#).unwrap());

static RE_POSTID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"class="postid">(\d+)</span>"#).unwrap());

static RE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)class="postusername">(.*?)</span>"#).unwrap());

static RE_TRAILING_A: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"</a>$"#).unwrap());

static RE_UNCLOSED_A_MAILTO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<a[^>]*href="mailto:[^"]*"[^>]*>"#).unwrap());

static RE_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"class="date">([^<]+)</span>"#).unwrap());

static RE_UID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"class="uid">([^<]+)</span>"#).unwrap());

static RE_CONTENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"class="post-content">(.*?)</div>"#).unwrap());

static RE_A_MAILTO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<a[^>]*href="mailto:[^"]*"[^>]*>(.*?)</a>"#).unwrap());

static RE_A_HREF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<a[^>]*>(.*?)</a>"#).unwrap());

static RE_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<h1[^>]*>([^<]+)</h1>"#).unwrap());

pub struct FetchDatParams {
    /// URL of the thread to fetch. Accepts either of the following formats:
    /// - dat URL: "https://{server}.5ch.io/{board}/dat/{thread_id}.dat"
    /// - read.cgi URL: "https://{server}.5ch.io/test/read.cgi/{board}/{thread_id}/"
    ///
    /// Regardless of the format, a direct dat download is always attempted first.
    /// If the dat returns 404 (aka dat落ち), read.cgi is used as a fallback.
    pub url: String,

    /// Destination file path. Accepts both absolute and relative paths.
    /// A relative path is resolved against the dat_dir specified via the CLI argument.
    pub save_path: String,
}

pub struct FetchDatResult {
    /// Absolute path of the saved dat file.
    pub save_path: String,

    /// Number of non-empty lines in the saved dat (i.e., the post count).
    pub res_count: usize,

    /// Increase in post count compared to the existing file.
    /// `None` if no file existed before; `Some(0)` if the file was already up to date.
    pub added_res_count: Option<usize>,
}

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

pub async fn fetch_dat(params: &FetchDatParams) -> Fallible<FetchDatResult> {
    let url_kind = parse_url(&params.url)?;

    let save_path = Path::new(&params.save_path);

    // Count existing posts before overwriting
    let existing_res_count = count_lines_if_exists(save_path);

    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15")
        .build()
        .context("HTTP クライアントの初期化に失敗しました")?;

    let ParsedUrl {
        server,
        board,
        thread_id,
    } = url_kind;

    // Try direct dat download first
    let dat_url = format!("https://{server}.5ch.io/{board}/dat/{thread_id}.dat");
    let dat_resp = client
        .get(&dat_url)
        .send()
        .await
        .with_context(|| format!("dat の取得に失敗しました: {dat_url}"))?;
    let dat_text = if dat_resp.status().is_success() {
        info!(url = %dat_url, "fetched dat directly");
        let bytes = dat_resp
            .bytes()
            .await
            .with_context(|| format!("dat のレスポンス読み取りに失敗しました: {dat_url}"))?;
        let (text, _, _) = encoding_rs::SHIFT_JIS.decode(&bytes);
        text.into_owned()
    } else if dat_resp.status() == reqwest::StatusCode::NOT_FOUND {
        // dat not found (dat落ち) — fall back to read.cgi
        let read_cgi_url = format!("https://{server}.5ch.io/test/read.cgi/{board}/{thread_id}/");
        info!(url = %read_cgi_url, "dat not found, falling back to read.cgi");
        let html_resp = client
            .get(&read_cgi_url)
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
        let (html, _, _) = encoding_rs::SHIFT_JIS.decode(&bytes);
        html_to_dat(&html)
    } else {
        bail!("dat取得失敗: {} ({})", dat_url, dat_resp.status());
    };

    let res_count = dat_text.lines().filter(|l| !l.is_empty()).count();

    if let Some(parent) = save_path.parent() {
        tokio::fs::create_dir_all(parent).await.with_context(|| {
            format!(
                "保存先ディレクトリの作成に失敗しました: {}",
                parent.display()
            )
        })?
    }
    tokio::fs::write(save_path, dat_text.as_bytes())
        .await
        .with_context(|| format!("ファイルの書き込みに失敗しました: {}", save_path.display()))?;

    let added_res_count = existing_res_count.map(|prev| res_count.saturating_sub(prev));

    Ok(FetchDatResult {
        save_path: params.save_path.clone(),
        res_count,
        added_res_count,
    })
}

/// Converts 5ch.io read.cgi HTML into dat format.
fn html_to_dat(html: &str) -> String {
    let thread_title = RE_TITLE
        .captures(html)
        .map(|c| c[1].trim_matches(|ch| ch == '\n' || ch == '\r').to_string())
        .unwrap_or_default();
    let mut lines = Vec::new();
    let posts: Vec<&str> = RE_POST.split(html).collect();
    for post in posts.iter().skip(1) {
        if !RE_POSTID.is_match(post) {
            continue;
        }

        let name = RE_NAME
            .captures(post)
            .map(|c| {
                let s = RE_A_MAILTO.replace_all(&c[1], "$1");
                let s = RE_TRAILING_A.replace(&s, "");
                RE_UNCLOSED_A_MAILTO.replace_all(&s, "").into_owned()
            })
            .unwrap_or_default();
        let date = RE_DATE
            .captures(post)
            .map(|c| c[1].trim().to_string())
            .unwrap_or_default();
        let uid = RE_UID
            .captures(post)
            .map(|c| c[1].trim().to_string())
            .unwrap_or_default();
        let body = RE_CONTENT
            .captures(post)
            .map(|c| RE_A_HREF.replace_all(&c[1], "$1").replace('"', "&quot;"))
            .unwrap_or_default();

        let datetime_id = if uid.is_empty() {
            date
        } else {
            format!("{date} {uid}")
        };

        // add title to res 1.
        let title = if lines.is_empty() {
            thread_title.as_str()
        } else {
            ""
        };
        lines.push(format!("{name}<><>{datetime_id}<>{body}<>{title}"));
    }
    lines.join("\n")
}

/// Returns the number of non-empty lines if the file exists, or None if it does not.
fn count_lines_if_exists(path: &Path) -> Option<usize> {
    let content = std::fs::read_to_string(path).ok()?;
    Some(content.lines().filter(|l| !l.is_empty()).count())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn html_to_dat_basic() {
        let html = r#"
<div class="clear post">
  <div class="post-header">
    <span class="postid">1</span>
    <span class="postusername"><b>名無しさん</b></span>
    <span class="date">2026/04/01(火) 12:00:00.00</span>
    <span class="uid">ID:abcdefgh</span>
  </div>
  <div class="post-content">テスト本文</div>
</div>
"#;
        let dat = html_to_dat(html);
        assert!(
            dat.contains(
                "<b>名無しさん</b><><>2026/04/01(火) 12:00:00.00 ID:abcdefgh<>テスト本文<>"
            )
        );
    }

    #[test]
    fn html_to_dat_no_uid() {
        let html = r#"
<div class="clear post">
  <div class="post-header">
    <span class="postid">1</span>
    <span class="postusername">名無し</span>
    <span class="date">2026/04/01(火) 12:00:00.00</span>
  </div>
  <div class="post-content">本文</div>
</div>
"#;
        let dat = html_to_dat(html);
        assert!(dat.contains("名無し<><>2026/04/01(火) 12:00:00.00<>本文<>"));
    }

    #[test]
    fn html_to_dat_mailto_name_and_anchor_body() {
        let html = r#"
<div class="clear post">
  <div class="post-header">
    <span class="postid">1</span>
    <span class="postusername"><b><a rel="nofollow" href="mailto:sage">名無しさん</a></b></span>
    <span class="date">2026/04/01(火) 12:00:00.00</span>
    <span class="uid">ID:abcdefgh</span>
  </div>
  <div class="post-content">詳細は <a href="http://jump5.ch/?https://example.com/" rel="nofollow" target="_blank">https://example.com/</a> を参照</div>
</div>
"#;
        let dat = html_to_dat(html);
        assert!(
            dat.contains(
                "<b>名無しさん</b><><>2026/04/01(火) 12:00:00.00 ID:abcdefgh<>詳細は https://example.com/ を参照<>"
            )
        );
    }

    // #[test]
    #[allow(unused)]
    fn wip_convert_html_to_dat_for_external_diff() {
        let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let html_path = workspace_root.join("actual.html");
        let dat_path = workspace_root.join("expected.dat");
        assert!(html_path.exists());
        assert!(dat_path.exists());

        let html_bytes = std::fs::read(&html_path).unwrap();
        let encoding = encoding_rs::SHIFT_JIS;
        let (html, _, _) = encoding.decode(&html_bytes);
        let result = html_to_dat(&html);

        let expected = std::fs::read_to_string(&dat_path).unwrap();
        let result_lines: Vec<&str> = result.lines().filter(|l| !l.is_empty()).collect();
        let expected_lines: Vec<&str> = expected.lines().filter(|l| !l.is_empty()).collect();

        std::fs::write("test_result.dat", result.as_bytes()).unwrap();

        assert_eq!(
            result_lines.len(),
            expected_lines.len(),
            "line count mismatch"
        );
        for (i, (r, e)) in result_lines.iter().zip(expected_lines.iter()).enumerate() {
            assert_eq!(r, e, "mismatch at line {}", i + 1);
        }
    }
}
