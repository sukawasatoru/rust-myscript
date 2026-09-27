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

//! Conversion of read.cgi HTML to dat text; no knowledge of retrieval or storage.

use regex::Regex;
use std::sync::LazyLock;

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

/// Converts 5ch.io read.cgi HTML into dat format.
pub(super) fn html_to_dat(html: &str) -> String {
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
        let title = if lines.is_empty() {
            thread_title.as_str()
        } else {
            ""
        };
        lines.push(format!("{name}<><>{datetime_id}<>{body}<>{title}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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
        assert!(dat.contains("<b>名無しさん</b><><>2026/04/01(火) 12:00:00.00 ID:abcdefgh<>詳細は https://example.com/ を参照<>"));
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
