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

use crate::model::{DatFileInfo, DatPost};
use regex::Regex;
use rust_myscript::prelude::*;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Safety limit for total response characters.
/// LM Studio truncates responses at ~54000 chars, so this provides a margin.
/// Applied when max_body_chars is 0 (no limit) or exceeds this value.
pub const MAX_BODY_CHARS_LIMIT: usize = 50000;

/// Estimated JSON structural overhead per row (`[`, `]`, `"`, `,`, etc.).
pub const ROW_OVERHEAD: usize = 30;

/// Returns the effective max_body_chars value. Clamps to the limit when 0 or exceeding.
fn effective_max_body_chars(value: usize) -> usize {
    if value == 0 || value > MAX_BODY_CHARS_LIMIT {
        MAX_BODY_CHARS_LIMIT
    } else {
        value
    }
}

/// Truncates items when cumulative character count exceeds the limit. Returns the omitted count.
/// `char_count` returns the text character count for each item. ROW_OVERHEAD is added internally.
///
/// Uses soft overflow: the item that *causes* the limit to be exceeded is still included,
/// so every call returns at least one item. Only items after the overflow point are omitted.
pub fn apply_cutoff<T>(
    items: &mut Vec<T>,
    max_body_chars: usize,
    char_count: impl Fn(&T) -> usize,
) -> usize {
    let effective_limit = effective_max_body_chars(max_body_chars);
    let mut accum = 0usize;
    let mut cutoff_idx = None;
    for (i, item) in items.iter().enumerate() {
        accum += char_count(item) + ROW_OVERHEAD;
        if accum > effective_limit {
            cutoff_idx = Some(i + 1);
            break;
        }
    }
    if let Some(idx) = cutoff_idx {
        let omitted = items.len() - idx;
        items.truncate(idx);
        omitted
    } else {
        0
    }
}

static RE_BR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>").unwrap());

static RE_HTML_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());

static URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"h?ttps?://[a-zA-Z0-9\-._~:/?#\[\]@!$&'()*+,;=%]+").unwrap());

static RE_ANCHOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&gt;&gt;(\d+)").unwrap());

/// 5ch infrastructure and ancillary service hosts to exclude from URL extraction.
/// - jump5.ch: 5ch's redirect proxy (wraps external links)
/// - 5ch.io/test: read.cgi thread links (internal navigation, not user content)
/// - seesaawiki, donguri, majinai: board-associated services (wikis, acorn system, etc.)
static EXCLUDED_HOSTS: &[&str] = &[
    "jump5.ch",
    "5ch.io/test",
    "seesaawiki",
    "donguri",
    "majinai",
];

/// Strips HTML tags and decodes entities from a dat line body.
pub fn clean_body(raw_body: &str) -> String {
    let s = RE_BR.replace_all(raw_body, "\n");
    let s = RE_HTML_TAG.replace_all(&s, "");
    s.replace("&gt;", ">")
        .replace("&lt;", "<")
        .replace("&amp;", "&")
        .replace("&#039;", "'")
        .trim()
        .to_string()
}

/// Splits a datetime-ID field (`"2026/03/13(金) 10:38:56.82 ID:abc"`) into datetime and ID.
pub fn parse_datetime_id(raw: &str) -> (String, String) {
    let trimmed = raw.trim();
    if let Some(idx) = trimmed.find(" ID:") {
        let datetime = trimmed[..idx].trim().to_string();
        let id = trimmed[idx + 1..].trim().to_string();
        (datetime, id)
    } else {
        (trimmed.to_string(), String::new())
    }
}

/// Parses a single dat line into a DatPost. Returns None on parse failure.
pub fn parse_dat_line(line: &str, res_num: usize) -> Option<DatPost> {
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

/// Resolves a file specifier (thread number "630" or filename) to an actual path.
pub fn resolve_dat_file(dat_dir: &Path, file_spec: &str) -> Fallible<PathBuf> {
    // Exact filename
    let direct = dat_dir.join(file_spec);
    if direct.is_file() {
        return Ok(direct);
    }

    // Thread number: find a file where parsed thread_num matches
    for entry in std::fs::read_dir(dat_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if let Some((num, _)) = parse_dat_filename(&name_str)
            && num.to_string() == file_spec
        {
            return Ok(entry.path());
        }
    }

    bail!("dat file not found for: {file_spec}")
}

/// Returns all .dat file paths in dat_dir (sorted by filename).
pub fn list_all_dat_files(dat_dir: &Path) -> Fallible<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dat_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.ends_with(".dat") && parse_dat_filename(&name_str).is_some() {
            files.push(entry.path());
        }
    }
    files.sort();
    Ok(files)
}

/// Extracts (thread_num, thread_id) from a filename.
/// Accepts `{prefix}_{num}_{id}.dat` format with any prefix.
pub fn parse_dat_filename(filename: &str) -> Option<(u32, String)> {
    let stem = filename.strip_suffix(".dat")?;
    // Split from the end: ..._{num}_{id}
    let mut parts = stem.rsplitn(3, '_');
    let id = parts.next()?;
    let num_str = parts.next()?;
    // Require a prefix part (at least 3 segments)
    parts.next()?;
    let num: u32 = num_str.parse().ok()?;
    Some((num, id.to_string()))
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

/// Reads a dat file and builds DatFileInfo.
pub fn build_file_info(path: &Path) -> Fallible<DatFileInfo> {
    let lines = read_lines(path)?;
    build_file_info_from_lines(path, &lines)
}

/// Builds DatFileInfo from pre-read lines.
pub fn build_file_info_from_lines(path: &Path, lines: &[String]) -> Fallible<DatFileInfo> {
    let filename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let (thread_num, thread_id) = parse_dat_filename(&filename).unwrap_or((0, String::new()));

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

    Ok(DatFileInfo {
        filename,
        thread_num,
        thread_id,
        total_lines,
        thread_title,
        date_range,
    })
}

/// Reads a dat file into lines.
pub fn read_lines(path: &Path) -> Fallible<Vec<String>> {
    let file = std::fs::File::open(path)?;
    let reader = BufReader::new(file);
    Ok(reader.lines().collect::<Result<_, _>>()?)
}

/// Converts a range string ("800-900", "800-", "-100") to a 1-based inclusive (start, end) pair.
pub fn resolve_range(range_str: &str, total_lines: usize) -> Fallible<(usize, usize)> {
    let range_str = range_str.trim();

    if let Some(last_n) = range_str.strip_prefix('-') {
        // "-100" -> last 100 posts
        let n: usize = last_n.parse().context("invalid range number")?;
        let start = if total_lines > n {
            total_lines - n + 1
        } else {
            1
        };
        return Ok((start, total_lines));
    }

    if let Some((left, right)) = range_str.split_once('-') {
        let start: usize = left.parse().context("invalid range start")?;
        if right.is_empty() {
            // "800-" -> from 800 to the end
            return Ok((start, total_lines));
        }
        let end: usize = right.parse().context("invalid range end")?;
        return Ok((start, end.min(total_lines)));
    }

    bail!("invalid range format: {range_str}")
}

/// Extracts URLs from text. Normalizes `ttp://` / `ttps://` to `http://` / `https://`.
///
/// The `ttp://` prefix is a 2ch/5ch convention where posters omit the leading `h`
/// to prevent auto-linking. We prepend `h` to restore valid URLs.
pub fn extract_urls(text: &str) -> Vec<String> {
    URL_RE
        .find_iter(text)
        .map(|m| {
            let url = m.as_str();
            if url.starts_with("ttp") && !url.starts_with("http") {
                format!("h{url}")
            } else {
                url.to_string()
            }
        })
        .filter(|u| !is_excluded_url(u))
        .collect()
}

/// Checks if a URL should be excluded.
pub fn is_excluded_url(url: &str) -> bool {
    EXCLUDED_HOSTS.iter().any(|host| url.contains(host))
}

/// Scans all posts in a thread and returns the reference count for each post number.
/// Anchors are stored as `&gt;&gt;N` in raw dat bodies (HTML-encoded `>>N`),
/// so we match the entity form rather than literal `>>`.
pub fn count_references(lines: &[String]) -> HashMap<usize, usize> {
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

/// Resolves file specifiers to actual file paths. Returns all files if empty.
pub fn resolve_files(dat_dir: &Path, files: &[String]) -> Fallible<Vec<PathBuf>> {
    if files.is_empty() {
        return list_all_dat_files(dat_dir);
    }

    let mut result = Vec::new();
    for spec in files {
        result.push(resolve_dat_file(dat_dir, spec)?);
    }
    result.sort();
    Ok(result)
}

#[cfg(test)]
pub mod test_helpers {
    use std::path::PathBuf;
    use tempfile::TempDir;

    pub struct TestDatDir {
        pub _dir: TempDir,
        pub dat_dir: PathBuf,
    }

    pub fn create_test_dat_dir() -> TestDatDir {
        let dir = TempDir::new().unwrap();
        let dat_dir = dir.path().join("dat_files");
        std::fs::create_dir_all(&dat_dir).unwrap();

        let dat_630 = [
            "テスト名<>sage<>2026/03/13(金) 10:38:56.82 ID:test0001<>最初のレス https://example.com/image001.jpg ここまで<>テストスレッド★630",
            "名無し<><>2026/03/13(金) 11:00:00.00 ID:test0002<>Tool v2.5すごい<br>&gt;&gt;1 これは便利<>",
            "名無し<>sage<>2026/03/13(金) 12:00:00.00 ID:test0003<>https://example.com/resources/12345 新しいプラグインが公開された<>",
            "名無し<><>2026/03/14(土) 09:00:00.00 ID:test0004<>App-X試してみた https://example.com/files/demo.mp4<>",
            "名無し<>sage<>2026/03/14(土) 10:00:00.00 ID:test0005<>https://example.com/repo/test Widget-Yも気になる<>",
        ].join("\n");
        std::fs::write(dat_dir.join("board_630_1773365936.dat"), &dat_630).unwrap();

        let dat_631 = [
            "テスト<>sage<>2026/03/18(水) 20:03:27.00 ID:test0010<>新スレ立てた<>テストスレッド★631",
            "名無し<><>2026/03/18(水) 21:00:00.00 ID:test0011<>Foobarで生成してみた https://example.com/output/xyz.png<>",
            "名無し<><>2026/03/18(水) 22:00:00.00 ID:test0012<>Bazqux<>",
        ].join("\n");
        std::fs::write(dat_dir.join("board_631_1773831807.dat"), &dat_631).unwrap();

        TestDatDir { _dir: dir, dat_dir }
    }
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
        assert_eq!(id, "ID:miLVJ0Bt0");
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
        assert_eq!(post.id, "ID:abc");
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
    fn parse_dat_filename_normal() {
        let (num, id) = parse_dat_filename("board_630_1773365936.dat").unwrap();
        assert_eq!(num, 630);
        assert_eq!(id, "1773365936");
    }

    #[test]
    fn parse_dat_filename_invalid() {
        assert!(parse_dat_filename("invalid.dat").is_none());
    }

    #[test]
    fn resolve_range_from_to() {
        assert_eq!(resolve_range("3-5", 10).unwrap(), (3, 5));
    }

    #[test]
    fn resolve_range_from() {
        assert_eq!(resolve_range("8-", 10).unwrap(), (8, 10));
    }

    #[test]
    fn resolve_range_last_n() {
        assert_eq!(resolve_range("-3", 10).unwrap(), (8, 10));
    }

    #[test]
    fn resolve_range_last_n_exceeds_total() {
        assert_eq!(resolve_range("-100", 5).unwrap(), (1, 5));
    }

    #[test]
    fn resolve_range_end_exceeds_total() {
        assert_eq!(resolve_range("1-999", 5).unwrap(), (1, 5));
    }

    #[test]
    fn extract_urls_basic() {
        let urls = extract_urls("check https://example.com/test.jpg and http://foo.bar/baz");
        assert_eq!(urls.len(), 2);
        assert!(urls[0].contains("example.com"));
    }

    #[test]
    fn extract_urls_excludes_jump5ch() {
        let urls = extract_urls("link http://jump5.ch/?https://real.url/test");
        assert!(urls.is_empty());
    }

    #[test]
    fn extract_urls_ttp_prefix() {
        let urls = extract_urls("check ttp://example.com/test and ttps://example.com/secure");
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0], "http://example.com/test");
        assert_eq!(urls[1], "https://example.com/secure");
    }

    #[test]
    fn extract_urls_mixed_http_and_ttp() {
        let urls = extract_urls("https://normal.com ttp://legacy.com");
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0], "https://normal.com");
        assert_eq!(urls[1], "http://legacy.com");
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
    fn resolve_dat_file_by_number() {
        let ctx = test_helpers::create_test_dat_dir();
        let path = resolve_dat_file(&ctx.dat_dir, "630").unwrap();
        assert!(path.to_string_lossy().contains("_630_"));
    }

    #[test]
    fn resolve_dat_file_by_name() {
        let ctx = test_helpers::create_test_dat_dir();
        let path = resolve_dat_file(&ctx.dat_dir, "board_630_1773365936.dat").unwrap();
        assert!(path.exists());
    }

    #[test]
    fn resolve_dat_file_not_found() {
        let ctx = test_helpers::create_test_dat_dir();
        assert!(resolve_dat_file(&ctx.dat_dir, "999").is_err());
    }

    #[test]
    fn list_all_dat_files_sorted() {
        let ctx = test_helpers::create_test_dat_dir();
        let files = list_all_dat_files(&ctx.dat_dir).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files[0].to_string_lossy().contains("630"));
        assert!(files[1].to_string_lossy().contains("631"));
    }

    #[test]
    fn build_file_info_excludes_over_1000_thread() {
        let dir = tempfile::TempDir::new().unwrap();
        let dat_dir = dir.path().join("dat_files");
        std::fs::create_dir_all(&dat_dir).unwrap();

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

        let content = lines.join("\n");
        std::fs::write(dat_dir.join("board_100_1234567890.dat"), &content).unwrap();

        let info = build_file_info(&dat_dir.join("board_100_1234567890.dat")).unwrap();

        // total_lines excludes "Over 1000 Thread"
        assert_eq!(info.total_lines, 1000);
        // date_range ends with the last post's datetime, not "Over 1000 Thread"
        assert!(!info.date_range.contains("Over 1000 Thread"));
        assert!(info.date_range.contains("2026/01/01"));
        // thread_title is from line 1
        assert_eq!(info.thread_title, "スレタイテスト");
    }

    #[test]
    fn build_file_info_normal_thread() {
        let ctx = test_helpers::create_test_dat_dir();
        let info = build_file_info(&ctx.dat_dir.join("board_630_1773365936.dat")).unwrap();
        assert_eq!(info.total_lines, 5);
        assert_eq!(info.thread_title, "テストスレッド★630");
        assert!(info.date_range.contains("2026/03/13"));
        assert!(info.date_range.contains("2026/03/14"));
    }
}
