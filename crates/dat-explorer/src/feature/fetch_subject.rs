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

//! Fetches a 5ch `subject.txt` (CP932) and returns it as structured UTF-8 data.

use regex::Regex;
use reqwest::{Client, Response, Url};
use rust_myscript::prelude::*;
use std::sync::LazyLock;
use std::time::Duration;

/// Fixed User-Agent. Kept constant so that the MCP client can allow-list this tool once.
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15";

/// Request timeout for a single `subject.txt` download.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum accepted body size (1 MiB). Exactly this size succeeds; one byte more fails.
const MAX_BODY_BYTES: u64 = 1024 * 1024;

/// Thread key of a `subject.txt` line, e.g. `1234567890.dat`.
///
/// `[0-9]` is used instead of `\d` on purpose: the `regex` crate's `\d` is Unicode-aware and
/// would also accept full-width digits such as `１２３`.
static RE_THREAD_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([0-9]+)\.dat$").unwrap());

/// Trailing res count of a `subject.txt` line, e.g. `  (634)`.
///
/// Notes:
/// - The `$` anchor is mandatory. Some real titles end with `(数字)` themselves
///   (`テストスレッド★633 (21146)  (36)`), so only the right-most parenthesised number is the res
///   count. Searching for `(` from the left, or dropping `$`, breaks those lines.
/// - `[0-9]` is used instead of `\d` for the same reason as [`RE_THREAD_KEY`].
/// - The separator is `\s+` rather than a fixed `  `. Measured over 52 boards / 20,382 lines the
///   separator was `"  "`, `" "`, `" \t "`, `"\t "` and full-width variants; a fixed `  `
///   separator failed on 48.5% of the lines.
/// - The `regex` crate's `\s` is Unicode-aware and therefore matches U+3000 (full-width space).
///   As a result a full-width space at the *head* of a title is kept while one at the *tail* is
///   consumed as part of the separator. The asymmetry is harmless because a separator only ever
///   exists on the tail side. Narrowing `\s+` to `[ \t]+` would break the 28+ observed lines that
///   use a full-width separator, so it stays `\s+`.
static RE_RES_COUNT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s+\(([0-9]+)\)\s*$").unwrap());

/// Numeric character reference, decimal (`&#129402;`) or hexadecimal (`&#x1F97A;` / `&#X1F97A;`).
///
/// Named entities such as `&amp;` are intentionally out of scope.
static RE_NCR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&#(?:[xX]([0-9a-fA-F]+)|([0-9]+));").unwrap());

pub struct FetchSubjectParams {
    /// URL of the `subject.txt` to fetch.
    ///
    /// Only `http` and `https` are accepted. userinfo is allowed (reqwest turns it into a Basic
    /// `Authorization` header), the fragment is stripped and the query is preserved.
    pub url: String,

    /// Case-insensitive substring filter applied to the NCR-restored title.
    ///
    /// `None` and an empty string both mean "no filtering".
    pub title_contains: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubjectThread {
    /// Thread key. Kept as a `String` so that leading zeros are preserved.
    pub thread_id: String,

    /// Thread title after NCR restoration. Not trimmed: leading spaces are part of the title.
    pub title: String,

    /// Number of posts in the thread.
    pub res_count: u64,
}

#[derive(Debug)]
pub struct FetchSubjectResult {
    /// Threads in the order they appear in `subject.txt`. Duplicates are preserved.
    pub threads: Vec<SubjectThread>,
}

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

/// Decodes CP932 bytes strictly.
///
/// The lossy [`encoding_rs::Encoding::decode`] is deliberately avoided: silently replacing broken
/// bytes with U+FFFD would hide a format change on the 5ch side.
fn decode_cp932(bytes: &[u8]) -> Fallible<String> {
    match encoding_rs::SHIFT_JIS.decode_without_bom_handling_and_without_replacement(bytes) {
        Some(text) => Ok(text.into_owned()),
        None => bail!("CP932 として解釈できないバイト列が含まれています"),
    }
}

/// Parses the whole `subject.txt` body.
///
/// Only completely empty lines are skipped; a whitespace-only line is a malformed line and is
/// reported as an error, because silently dropping it would hide a format change and would be
/// inconsistent with not trimming titles.
fn parse_subject(text: &str) -> Fallible<Vec<SubjectThread>> {
    let mut threads = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let line_no = index + 1;
        threads.push(parse_line(line).with_context(|| format!("{line_no} 行目"))?);
    }
    Ok(threads)
}

fn parse_line(line: &str) -> Fallible<SubjectThread> {
    let Some((key, rest)) = line.split_once("<>") else {
        bail!("セパレーター \"<>\" が見つかりません");
    };

    let Some(caps) = RE_THREAD_KEY.captures(key) else {
        bail!("スレッドキーの形式が不正です: {key:?}");
    };
    let thread_id = caps[1].to_owned();

    let Some(caps) = RE_RES_COUNT.captures(rest) else {
        bail!("レス数が見つかりません: {rest:?}");
    };
    let res_count = caps[1]
        .parse::<u64>()
        .with_context(|| format!("レス数を数値に変換できません: {:?}", &caps[1]))?;

    let separator_start = caps.get(0).expect("group 0 always exists").start();
    let title = &rest[..separator_start];
    if title.is_empty() {
        bail!("スレッドタイトルが空です");
    }

    Ok(SubjectThread {
        thread_id,
        title: restore_ncr(title),
        res_count,
    })
}

/// Restores decimal and hexadecimal numeric character references.
///
/// Invalid scalar values (surrogates, out-of-range, overflowing) and unterminated references are
/// left as they are, matching the behaviour of [`crate::dat::clean_body`].
fn restore_ncr(s: &str) -> String {
    RE_NCR
        .replace_all(s, |caps: &regex::Captures| {
            let code_point = match caps.get(1) {
                Some(hex) => u32::from_str_radix(hex.as_str(), 16).ok(),
                None => caps[2].parse::<u32>().ok(),
            };
            code_point
                .and_then(char::from_u32)
                .map_or_else(|| caps[0].to_owned(), |c| c.to_string())
        })
        .into_owned()
}

/// Applies the `title_contains` filter.
///
/// Case-insensitivity is simple `to_lowercase` + `contains` on both sides; full Unicode case
/// folding and normalization are out of scope.
fn filter_threads(threads: Vec<SubjectThread>, title_contains: Option<&str>) -> Vec<SubjectThread> {
    let needle = match title_contains {
        Some(needle) if !needle.is_empty() => needle.to_lowercase(),
        _ => return threads,
    };

    threads
        .into_iter()
        .filter(|thread| thread.title.to_lowercase().contains(&needle))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(thread_id: &str, title: &str, res_count: u64) -> SubjectThread {
        SubjectThread {
            thread_id: thread_id.to_owned(),
            title: title.to_owned(),
            res_count,
        }
    }

    #[test]
    fn parse_subject_basic() {
        let actual = parse_subject("1234567890.dat<>テストスレッド★630  (649)").unwrap();
        assert_eq!(
            actual,
            vec![thread("1234567890", "テストスレッド★630", 649)]
        );
    }

    #[test]
    fn parse_subject_single_space_separator() {
        let actual = parse_subject("1234567890.dat<>タイトル (1)").unwrap();
        assert_eq!(actual, vec![thread("1234567890", "タイトル", 1)]);
    }

    #[test]
    fn parse_subject_many_space_separator() {
        let actual = parse_subject("1234567890.dat<>タイトル     (12)").unwrap();
        assert_eq!(actual, vec![thread("1234567890", "タイトル", 12)]);
    }

    #[test]
    fn parse_subject_tab_separator() {
        let actual = parse_subject("1.dat<>A \t (2)\n2.dat<>B\t (3)\n3.dat<>C\t(4)").unwrap();
        assert_eq!(
            actual,
            vec![
                thread("1", "A", 2),
                thread("2", "B", 3),
                thread("3", "C", 4)
            ]
        );
    }

    #[test]
    fn parse_subject_fullwidth_separator() {
        let actual = parse_subject("1234567890.dat<>タイトル　  (5)").unwrap();
        assert_eq!(actual, vec![thread("1234567890", "タイトル", 5)]);
    }

    #[test]
    fn parse_subject_keeps_leading_half_and_full_width_space() {
        let actual =
            parse_subject("1234567894.dat<> ★ テスト告知スレッド (2)\n1.dat<>　全角先頭  (3)")
                .unwrap();
        assert_eq!(
            actual,
            vec![
                thread("1234567894", " ★ テスト告知スレッド", 2),
                thread("1", "　全角先頭", 3),
            ]
        );
    }

    #[test]
    fn parse_subject_consumes_only_trailing_separator_space() {
        let actual = parse_subject("1.dat<>　タイトル　  (10)").unwrap();
        assert_eq!(actual, vec![thread("1", "　タイトル", 10)]);
    }

    #[test]
    fn parse_subject_title_contains_parentheses() {
        let actual = parse_subject("1.dat<>雑談(実況)スレ  (42)").unwrap();
        assert_eq!(actual, vec![thread("1", "雑談(実況)スレ", 42)]);
    }

    #[test]
    fn parse_subject_title_ends_with_parenthesised_number() {
        let actual = parse_subject(
            "1.dat<>テストスレッド (11)  (307)\n2.dat<>テストスレッド★633 (21146)  (36)",
        )
        .unwrap();
        assert_eq!(
            actual,
            vec![
                thread("1", "テストスレッド (11)", 307),
                thread("2", "テストスレッド★633 (21146)", 36),
            ]
        );
    }

    #[test]
    fn parse_subject_keeps_be_id_in_title() {
        let actual =
            parse_subject("1234567890.dat<>タイトル [BE:1234567890-2BP(1000)]  (634)").unwrap();
        assert_eq!(
            actual,
            vec![thread(
                "1234567890",
                "タイトル [BE:1234567890-2BP(1000)]",
                634
            )]
        );
    }

    #[test]
    fn parse_subject_keeps_leading_zero_thread_id() {
        let actual = parse_subject("0000000000.dat<>タイトル  (7)").unwrap();
        assert_eq!(actual, vec![thread("0000000000", "タイトル", 7)]);
    }

    #[test]
    fn parse_subject_lf() {
        let actual = parse_subject("1.dat<>A  (1)\n2.dat<>B  (2)\n").unwrap();
        assert_eq!(actual, vec![thread("1", "A", 1), thread("2", "B", 2)]);
    }

    #[test]
    fn parse_subject_crlf() {
        let actual = parse_subject("1.dat<>A  (1)\r\n2.dat<>B  (2)\r\n").unwrap();
        assert_eq!(actual, vec![thread("1", "A", 1), thread("2", "B", 2)]);
    }

    #[test]
    fn parse_subject_without_trailing_newline() {
        let actual = parse_subject("1.dat<>A  (1)\n2.dat<>B  (2)").unwrap();
        assert_eq!(actual, vec![thread("1", "A", 1), thread("2", "B", 2)]);
    }

    #[test]
    fn parse_subject_keeps_order_and_duplicates() {
        let actual = parse_subject("3.dat<>C  (3)\n1.dat<>A  (1)\n3.dat<>C  (3)").unwrap();
        assert_eq!(
            actual,
            vec![
                thread("3", "C", 3),
                thread("1", "A", 1),
                thread("3", "C", 3),
            ]
        );
    }

    #[test]
    fn parse_subject_zero_res_count() {
        let actual = parse_subject("1.dat<>タイトル  (0)").unwrap();
        assert_eq!(actual, vec![thread("1", "タイトル", 0)]);
    }

    #[test]
    fn parse_subject_max_u64_res_count() {
        let actual = parse_subject("1.dat<>タイトル  (18446744073709551615)").unwrap();
        assert_eq!(actual, vec![thread("1", "タイトル", u64::MAX)]);
    }

    #[test]
    fn parse_subject_empty_body() {
        assert_eq!(parse_subject("").unwrap(), vec![]);
        assert_eq!(parse_subject("\n").unwrap(), vec![]);
        assert_eq!(parse_subject("\r\n").unwrap(), vec![]);
    }

    #[test]
    fn parse_subject_ignores_empty_lines_only() {
        assert_eq!(parse_subject("\n\n\n").unwrap(), vec![]);
        let actual = parse_subject("\n1.dat<>A  (1)\n\n2.dat<>B  (2)\n\n").unwrap();
        assert_eq!(actual, vec![thread("1", "A", 1), thread("2", "B", 2)]);
    }

    fn parse_err(text: &str) -> String {
        format!("{:#}", parse_subject(text).unwrap_err())
    }

    #[test]
    fn parse_subject_without_separator() {
        let actual = parse_err("1.dat 単なる文字列 (1)");
        assert!(actual.contains("1 行目"), "{actual}");
        assert!(actual.contains("<>"), "{actual}");
    }

    #[test]
    fn parse_subject_without_dat_suffix() {
        let actual = parse_err("1.dat<>A  (1)\n1234567890<>B  (2)");
        assert!(actual.contains("2 行目"), "{actual}");
    }

    #[test]
    fn parse_subject_non_ascii_digit_thread_id() {
        let actual = parse_err("１２３.dat<>A  (1)");
        assert!(actual.contains("1 行目"), "{actual}");

        let actual = parse_err("abc.dat<>A  (1)");
        assert!(actual.contains("1 行目"), "{actual}");
    }

    #[test]
    fn parse_subject_without_res_count() {
        let actual = parse_err("1.dat<>タイトルのみ");
        assert!(actual.contains("1 行目"), "{actual}");

        let actual = parse_err("1.dat<>タイトル  (12)x");
        assert!(actual.contains("1 行目"), "{actual}");
    }

    #[test]
    fn parse_subject_non_ascii_digit_res_count() {
        let actual = parse_err("1.dat<>タイトル  (１２)");
        assert!(actual.contains("1 行目"), "{actual}");

        let actual = parse_err("1.dat<>タイトル  (abc)");
        assert!(actual.contains("1 行目"), "{actual}");
    }

    #[test]
    fn parse_subject_res_count_overflow() {
        let actual = parse_err("1.dat<>タイトル  (18446744073709551616)");
        assert!(actual.contains("1 行目"), "{actual}");
        assert!(actual.contains("レス数"), "{actual}");
    }

    #[test]
    fn parse_subject_empty_title() {
        let actual = parse_err("1.dat<> (1)");
        assert!(actual.contains("1 行目"), "{actual}");
        assert!(actual.contains("タイトル"), "{actual}");
    }

    #[test]
    fn parse_subject_whitespace_only_line() {
        let actual = parse_err("1.dat<>A  (1)\n   \n2.dat<>B  (2)");
        assert!(actual.contains("2 行目"), "{actual}");
    }

    #[test]
    fn parse_subject_error_line_number_counts_empty_lines() {
        let actual = parse_err("\n\n1.dat<>A  (1)\n\nbroken");
        assert!(actual.contains("5 行目"), "{actual}");
    }

    #[test]
    fn restore_ncr_decimal() {
        assert_eq!(
            restore_ncr("&#129402;絵文字テストスレッド"),
            "🥺絵文字テストスレッド"
        );
    }

    #[test]
    fn restore_ncr_hex_lowercase() {
        assert_eq!(restore_ncr("&#x1F97A;"), "🥺");
        assert_eq!(restore_ncr("&#x1f97a;"), "🥺");
    }

    #[test]
    fn restore_ncr_hex_uppercase_marker() {
        assert_eq!(restore_ncr("&#X1F97A;"), "🥺");
    }

    #[test]
    fn restore_ncr_supplementary_plane() {
        assert_eq!(restore_ncr("&#131083;"), "\u{2000B}");
    }

    #[test]
    fn restore_ncr_multiple_mixed() {
        assert_eq!(
            restore_ncr("&#129402;テスト&#x1F97A;スレッド&#65;"),
            "🥺テスト🥺スレッドA"
        );
    }

    #[test]
    fn restore_ncr_keeps_surrogate() {
        assert_eq!(restore_ncr("&#55296;"), "&#55296;");
        assert_eq!(restore_ncr("&#xD800;"), "&#xD800;");
    }

    #[test]
    fn restore_ncr_keeps_out_of_range() {
        assert_eq!(restore_ncr("&#1114112;"), "&#1114112;");
        assert_eq!(restore_ncr("&#x110000;"), "&#x110000;");
        assert_eq!(restore_ncr("&#99999999999999;"), "&#99999999999999;");
        assert_eq!(restore_ncr("&#xFFFFFFFFFF;"), "&#xFFFFFFFFFF;");
    }

    #[test]
    fn restore_ncr_keeps_unterminated_and_invalid() {
        assert_eq!(restore_ncr("&#129402"), "&#129402");
        assert_eq!(restore_ncr("&#;"), "&#;");
        assert_eq!(restore_ncr("&#x;"), "&#x;");
        assert_eq!(restore_ncr("&#abc;"), "&#abc;");
        assert_eq!(restore_ncr("A&amp;B"), "A&amp;B");
    }

    #[test]
    fn parse_subject_restores_ncr_in_title() {
        let actual =
            parse_subject("1234567892.dat<>&#129402;絵文字テストスレッド&#129402;★631  (398)")
                .unwrap();
        assert_eq!(
            actual,
            vec![thread("1234567892", "🥺絵文字テストスレッド🥺★631", 398)]
        );
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

    #[test]
    fn decode_cp932_ascii() {
        assert_eq!(
            decode_cp932(b"1.dat<>title  (1)\n").unwrap(),
            "1.dat<>title  (1)\n"
        );
    }

    #[test]
    fn decode_cp932_japanese() {
        let (bytes, _, unmappable) = encoding_rs::SHIFT_JIS.encode("テストスレッド★630");
        assert!(!unmappable);
        assert_eq!(decode_cp932(&bytes).unwrap(), "テストスレッド★630");
    }

    #[test]
    fn decode_cp932_rejects_invalid_bytes() {
        assert!(decode_cp932(&[0x81, 0x20]).is_err());
        assert!(decode_cp932(&[0x82, 0xA0, 0x81]).is_err());
    }

    #[test]
    fn decode_cp932_empty() {
        assert_eq!(decode_cp932(b"").unwrap(), "");
    }

    fn sample_threads() -> Vec<SubjectThread> {
        vec![
            thread("1", "日本語テストスレッド★630", 649),
            thread("2", "Test Thread Alpha", 100),
            thread("3", "🥺絵文字テストスレッド🥺★631", 398),
            thread("4", "İtest テストスレッド★632", 5),
        ]
    }

    #[test]
    fn filter_threads_none_returns_all() {
        assert_eq!(filter_threads(sample_threads(), None), sample_threads());
    }

    #[test]
    fn filter_threads_empty_needle_returns_all() {
        assert_eq!(filter_threads(sample_threads(), Some("")), sample_threads());
    }

    #[test]
    fn filter_threads_japanese() {
        let actual = filter_threads(sample_threads(), Some("日本語"));
        assert_eq!(actual, vec![thread("1", "日本語テストスレッド★630", 649)]);
    }

    #[test]
    fn filter_threads_ascii_case_insensitive() {
        let actual = filter_threads(sample_threads(), Some("TEST thread"));
        assert_eq!(actual, vec![thread("2", "Test Thread Alpha", 100)]);
    }

    #[test]
    fn filter_threads_matches_restored_ncr_title() {
        let actual = filter_threads(sample_threads(), Some("🥺"));
        assert_eq!(
            actual,
            vec![thread("3", "🥺絵文字テストスレッド🥺★631", 398)]
        );
        assert!(filter_threads(sample_threads(), Some("&#129402;")).is_empty());
    }

    #[test]
    fn filter_threads_multi_code_point_lowercase() {
        let actual = filter_threads(sample_threads(), Some("İTEST"));
        assert_eq!(actual, vec![thread("4", "İtest テストスレッド★632", 5)]);

        let actual = filter_threads(sample_threads(), Some("İtest"));
        assert_eq!(actual, vec![thread("4", "İtest テストスレッド★632", 5)]);
    }

    #[test]
    fn filter_threads_no_hit() {
        assert!(filter_threads(sample_threads(), Some("存在しない")).is_empty());
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
