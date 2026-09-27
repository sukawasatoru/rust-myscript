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

use crate::model::fetch::SubjectThread;
use crate::model::{DatFileInfo, DatPost};
use regex::Regex;
use rust_myscript::prelude::*;
use std::collections::HashMap;

/// Case-insensitive title substring match on decoded threads. Full Unicode case folding
/// and normalization are out of scope; both sides use `to_lowercase`.
pub fn filter_threads(
    threads: Vec<SubjectThread>,
    title_contains: Option<&str>,
) -> Vec<SubjectThread> {
    let needle = match title_contains {
        Some(needle) if !needle.is_empty() => needle.to_lowercase(),
        _ => return threads,
    };
    threads
        .into_iter()
        .filter(|thread| thread.title.to_lowercase().contains(&needle))
        .collect()
}

/// Converts "800-900", "800-", or "-100" to a 1-based inclusive range.
pub fn resolve_range(range_str: &str, total_lines: usize) -> Fallible<(usize, usize)> {
    let range_str = range_str.trim();
    if let Some(last_n) = range_str.strip_prefix('-') {
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
            return Ok((start, total_lines));
        }
        let end: usize = right.parse().context("invalid range end")?;
        return Ok((start, end.min(total_lines)));
    }
    bail!("invalid range format: {range_str}")
}

#[derive(Default)]
pub struct ReadPostsQuery {
    pub file: String,
    pub range: Option<String>,
    /// Specific post numbers, overriding range.
    pub res_nums: Vec<usize>,
    pub include_urls: bool,
}

/// Unbudgeted read result; output limits belong to the caller.
pub struct ReadPostsResult {
    pub posts: Vec<DatPost>,
    pub file_info: DatFileInfo,
    pub ref_counts: HashMap<usize, usize>,
    pub urls: HashMap<usize, Vec<String>>,
}

#[derive(Default)]
pub struct SearchPostsQuery {
    pub keywords: Vec<String>,
    pub files: Vec<String>,
    pub range: Option<String>,
    pub ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub file: String,
    pub res_num: usize,
    pub datetime: String,
    pub id: String,
    pub body: String,
    pub urls: Vec<String>,
    pub ref_count: usize,
}

pub struct SearchPostsResult {
    pub hits: Vec<SearchHit>,
    pub searched_files: Vec<String>,
}

/// A compiled query over parsed posts, independent of storage and transport.
pub struct PostMatcher {
    keywords: Vec<Regex>,
    ids: Vec<String>,
}

impl PostMatcher {
    pub fn new(query: &SearchPostsQuery) -> Fallible<Self> {
        ensure!(
            !query.keywords.is_empty() || !query.ids.is_empty(),
            "keywords または ids を指定してください"
        );
        let keywords = query
            .keywords
            .iter()
            .map(|kw| {
                Regex::new(&format!("(?i){kw}")).with_context(|| format!("invalid regex: {kw}"))
            })
            .collect::<Fallible<_>>()?;
        Ok(Self {
            keywords,
            ids: query.ids.clone(),
        })
    }

    pub fn matches(&self, post: &DatPost) -> bool {
        (self.ids.is_empty() || self.ids.iter().any(|id| post.id.contains(id)))
            && (self.keywords.is_empty() || self.keywords.iter().any(|re| re.is_match(&post.body)))
    }
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
}
