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

//! Parsing of dat text, independent of its source or storage filename.

use crate::model::{DatContentInfo, DatPost};
use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

static RE_BR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>").unwrap());

static RE_HTML_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());

static RE_NUMERIC_REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&#([0-9]+);").unwrap());

static RE_ANCHOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&gt;&gt;(\d+)").unwrap());

/// Strips HTML tags and decodes entities from a dat line body.
fn clean_body(raw_body: &str) -> String {
    let s = RE_BR.replace_all(raw_body, "\n");
    let s = RE_HTML_TAG.replace_all(&s, "");
    let s = s.replace("&gt;", ">").replace("&lt;", "<");
    let s = RE_NUMERIC_REF.replace_all(&s, |caps: &regex::Captures| {
        caps[1]
            .parse::<u32>()
            .ok()
            .and_then(char::from_u32)
            .map_or_else(|| caps[0].to_string(), |c| c.to_string())
    });
    s.replace("&amp;", "&").trim().to_string()
}

/// Splits a datetime-ID field (`"2026/03/13(金) 10:38:56.82 ID:abc"`) into datetime and ID.
fn parse_datetime_id(raw: &str) -> (String, String) {
    let trimmed = raw.trim();
    if let Some(idx) = trimmed.find(" ID:") {
        let datetime = trimmed[..idx].trim().to_string();
        // skip " ID:" prefix (4 chars) to extract the bare ID
        let id = trimmed[idx + 4..].trim().to_string();
        (datetime, id)
    } else {
        (trimmed.to_string(), String::new())
    }
}

/// Parses a single dat line into a DatPost. Returns None on parse failure.
pub(super) fn parse_dat_line(line: &str, res_num: usize) -> Option<DatPost> {
    let parts: Vec<&str> = line.split("<>").collect();
    if parts.len() < 4 {
        return None;
    }

    let name = RE_HTML_TAG.replace_all(parts[0], "").to_string();
    let mail = parts[1].to_string();
    let (datetime, id) = parse_datetime_id(parts[2]);
    let body = clean_body(parts[3]);
    let title = if parts.len() >= 5 && !parts[4].is_empty() {
        Some(parts[4].to_string())
    } else {
        None
    };

    Some(DatPost {
        res_num,
        name,
        mail,
        datetime,
        id,
        body,
        title,
    })
}

/// Checks whether a dat line is a valid post line.
/// Requires at least 4 `<>`-delimited fields and a valid datetime field.
fn is_valid_dat_line(line: &str) -> bool {
    line.split("<>").count() >= 4 && extract_datetime(line).is_some()
}

/// Extracts the datetime from a dat line. Returns None for non-post lines.
fn extract_datetime(line: &str) -> Option<String> {
    let parts: Vec<&str> = line.split("<>").collect();
    if parts.len() < 3 {
        return None;
    }
    let dt = parse_datetime_id(parts[2]).0;
    // Basic validation: starts with a digit and contains "/"
    // (excludes special lines like "Over 1000 Thread")
    if dt.len() >= 5 && dt.starts_with(|c: char| c.is_ascii_digit()) && dt.contains('/') {
        Some(dt)
    } else {
        None
    }
}

/// Extracts content metadata without interpreting a storage path or filename.
pub(super) fn parse_content_info(lines: &[String]) -> DatContentInfo {
    // Count only valid post lines (excludes "Over 1000 Thread" etc.)
    let total_lines = lines.iter().filter(|l| is_valid_dat_line(l)).count();

    let thread_title = lines
        .first()
        .and_then(|line| {
            let parts: Vec<&str> = line.split("<>").collect();
            if parts.len() >= 5 && !parts[4].is_empty() {
                Some(parts[4].trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_default();

    let first_dt = lines
        .first()
        .and_then(|l| extract_datetime(l))
        .unwrap_or_default();

    // Scan from the end to find the last valid post's datetime
    let last_dt = lines
        .iter()
        .rev()
        .find_map(|l| extract_datetime(l))
        .unwrap_or_default();

    let date_range = if first_dt.is_empty() && last_dt.is_empty() {
        String::new()
    } else {
        format!("{first_dt} - {last_dt}")
    };

    DatContentInfo {
        total_lines,
        thread_title,
        date_range,
    }
}

/// Scans all posts in a thread and returns the reference count for each post number.
/// Anchors are stored as `&gt;&gt;N` in raw dat bodies (HTML-encoded `>>N`),
/// so we match the entity form rather than literal `>>`.
pub(super) fn count_references(lines: &[String]) -> HashMap<usize, usize> {
    let mut counts = HashMap::new();
    for line in lines {
        let parts: Vec<&str> = line.split("<>").collect();
        if parts.len() < 4 {
            continue;
        }
        for cap in RE_ANCHOR.captures_iter(parts[3]) {
            if let Ok(n) = cap[1].parse::<usize>() {
                *counts.entry(n).or_insert(0) += 1;
            }
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_body_basic() {
        assert_eq!(clean_body("hello<br>world"), "hello\nworld");
    }

    #[test]
    fn clean_body_entities() {
        assert_eq!(clean_body("a &gt; b &lt; c &amp; d"), "a > b < c & d");
    }

    #[test]
    fn clean_body_numeric_ref() {
        assert_eq!(clean_body("&#039;hello&#039;"), "'hello'");
        assert_eq!(clean_body("&#34;quoted&#34;"), "\"quoted\"");
    }

    #[test]
    fn clean_body_numeric_ref_unicode() {
        assert_eq!(clean_body("&#12354;"), "あ");
    }

    #[test]
    fn clean_body_numeric_ref_invalid() {
        assert_eq!(clean_body("&#99999999;"), "&#99999999;");
    }

    #[test]
    fn clean_body_strip_tags() {
        assert_eq!(
            clean_body("<b>bold</b> <a href=\"x\">link</a>"),
            "bold link"
        );
    }

    #[test]
    fn clean_body_br_variants() {
        assert_eq!(clean_body("a<br>b<BR>c<br/>d<br />e"), "a\nb\nc\nd\ne");
    }

    #[test]
    fn parse_datetime_id_normal() {
        let (dt, id) = parse_datetime_id("2026/03/13(金) 10:38:56.82 ID:miLVJ0Bt0");
        assert_eq!(dt, "2026/03/13(金) 10:38:56.82");
        assert_eq!(id, "miLVJ0Bt0");
    }

    #[test]
    fn parse_datetime_id_no_id() {
        let (dt, id) = parse_datetime_id("2026/03/13(金) 10:38:56.82");
        assert_eq!(dt, "2026/03/13(金) 10:38:56.82");
        assert_eq!(id, "");
    }

    #[test]
    fn parse_dat_line_normal() {
        let line = "名前<>sage<>2026/03/13(金) 10:38:56.82 ID:abc<>本文テスト<>スレタイ";
        let post = parse_dat_line(line, 1).unwrap();
        assert_eq!(post.res_num, 1);
        assert_eq!(post.name, "名前");
        assert_eq!(post.mail, "sage");
        assert_eq!(post.datetime, "2026/03/13(金) 10:38:56.82");
        assert_eq!(post.id, "abc");
        assert_eq!(post.body, "本文テスト");
        assert_eq!(post.title.as_deref(), Some("スレタイ"));
    }

    #[test]
    fn parse_dat_line_no_title() {
        let line = "名前<><>2026/03/13(金) 11:00:00.00 ID:xyz<>body<>";
        let post = parse_dat_line(line, 2).unwrap();
        assert_eq!(post.res_num, 2);
        assert!(post.title.is_none());
    }

    #[test]
    fn parse_dat_line_too_few_fields() {
        let line = "a<>b<>c";
        assert!(parse_dat_line(line, 1).is_none());
    }

    #[test]
    fn parse_dat_line_clean_name_tags() {
        let line =
            "<b>名無し</b><small>（ﾜｯﾁｮｲ）</small><>sage<>2026/03/13(金) 11:00:00.00 ID:x<>body<>";
        let post = parse_dat_line(line, 1).unwrap();
        assert_eq!(post.name, "名無し（ﾜｯﾁｮｲ）");
    }

    #[test]
    fn count_references_basic() {
        let lines = vec![
            "名前<><>2026/01/01(水) 00:00:00.00 ID:a<>テスト<>スレタイ".to_string(),
            "名前<><>2026/01/01(水) 00:01:00.00 ID:b<>&gt;&gt;1 すごい<>".to_string(),
            "名前<><>2026/01/01(水) 00:02:00.00 ID:c<>&gt;&gt;1 &gt;&gt;2 同意<>".to_string(),
        ];
        let counts = count_references(&lines);
        assert_eq!(counts.get(&1), Some(&2)); // res 1 referenced 2 times
        assert_eq!(counts.get(&2), Some(&1)); // res 2 referenced 1 time
        assert_eq!(counts.get(&3), None); // res 3 not referenced
    }

    #[test]
    fn count_references_no_anchors() {
        let lines =
            vec!["名前<><>2026/01/01(水) 00:00:00.00 ID:a<>アンカーなし<>スレタイ".to_string()];
        let counts = count_references(&lines);
        assert!(counts.is_empty());
    }

    #[test]
    fn parse_content_info_excludes_over_1000_thread() {
        let mut lines: Vec<String> =
            vec!["名前<>sage<>2026/01/01(水) 00:00:00.00 ID:aaa<>レス1<>スレタイテスト".into()];
        for i in 2..=1000 {
            lines.push(format!(
                "名無し<><>2026/01/01(水) {:02}:{:02}:00.00 ID:x{:04}<>レス{}<>",
                i / 60,
                i % 60,
                i,
                i
            ));
        }
        lines.push("Over 1000 Thread".into());

        let info = parse_content_info(&lines);

        // total_lines excludes "Over 1000 Thread"
        assert_eq!(info.total_lines, 1000);
        // date_range ends with the last post's datetime, not "Over 1000 Thread"
        assert!(!info.date_range.contains("Over 1000 Thread"));
        assert!(info.date_range.contains("2026/01/01"));
        // thread_title is from line 1
        assert_eq!(info.thread_title, "スレタイテスト");
    }
}
