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

use crate::dat;
use regex::Regex;
use rust_myscript::prelude::*;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Default)]
pub struct SearchPostsParams {
    /// Search keywords (regex).
    pub keywords: Vec<String>,
    pub files: Vec<String>,
    pub range: Option<String>,
    /// Filter by poster ID (partial match). Empty means no filter.
    pub ids: Vec<String>,
    /// Approximate upper limit for cumulative text characters of hits. 0 = no limit.
    pub max_body_chars: usize,
    /// Whether the id field is included in the response.
    /// Affects cutoff calculation: excluded id chars are not counted.
    pub include_id: bool,
    /// When true, the safety cap (MAX_BODY_CHARS_LIMIT) is not applied.
    pub disable_body_limit: bool,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub file: String,
    pub res_num: usize,
    pub datetime: String,
    pub id: String,
    pub body: String,
    pub urls: Vec<String>,
    /// Reference count for this post (>>N anchor aggregation)
    pub ref_count: usize,
}

impl SearchHit {
    /// Returns the estimated character count for response fields.
    pub fn response_chars(&self, include_id: bool) -> usize {
        let id_chars = if include_id {
            self.id.chars().count()
        } else {
            0
        };
        self.file.chars().count()
            + self.datetime.chars().count()
            + id_chars
            + self.body.chars().count()
            + self.urls.iter().map(|u| u.chars().count()).sum::<usize>()
    }
}

pub struct SearchPostsResult {
    pub hits: Vec<SearchHit>,
    pub total_hits: usize,
    pub searched_files: Vec<String>,
    /// Number of hits omitted due to max_body_chars exceeded
    pub omitted_count: usize,
}

pub fn search_posts(dat_dir: &Path, params: &SearchPostsParams) -> Fallible<SearchPostsResult> {
    ensure!(
        !params.keywords.is_empty() || !params.ids.is_empty(),
        "keywords または ids を指定してください"
    );

    let compiled: Vec<(String, Regex)> = params
        .keywords
        .iter()
        .map(|kw| {
            Regex::new(&format!("(?i){kw}"))
                .map(|re| (kw.clone(), re))
                .with_context(|| format!("invalid regex: {kw}"))
        })
        .collect::<Fallible<_>>()?;

    let paths = dat::resolve_files(dat_dir, &params.files)?;
    let mut hits = Vec::new();
    let mut searched_files = Vec::new();

    for path in &paths {
        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        searched_files.push(filename.clone());

        let file = std::fs::File::open(path)?;
        let reader = BufReader::new(file);
        let lines: Vec<String> = reader.lines().collect::<Result<_, _>>()?;
        let total = lines.len();
        let ref_counts = dat::count_references(&lines);

        let (start, end) = if let Some(ref range_str) = params.range {
            dat::resolve_range(range_str, total)?
        } else {
            (1, total)
        };

        for (i, line) in lines.iter().enumerate() {
            let res_num = i + 1;
            if res_num < start || res_num > end {
                continue;
            }

            let post = match dat::parse_dat_line(line, res_num) {
                Some(p) => p,
                None => continue,
            };

            // ID filter
            if !params.ids.is_empty() {
                let id_matched = params.ids.iter().any(|id| post.id.contains(id));
                if !id_matched {
                    continue;
                }
            }

            // Keyword matching (empty keywords matches all posts)
            let matched: Vec<String> = if compiled.is_empty() {
                vec!["*".to_string()]
            } else {
                compiled
                    .iter()
                    .filter(|(_, re)| re.is_match(&post.body))
                    .map(|(kw, _)| kw.clone())
                    .collect()
            };

            if matched.is_empty() {
                continue;
            }

            let urls = dat::extract_urls(&post.body);

            let ref_count = ref_counts.get(&res_num).copied().unwrap_or(0);
            hits.push(SearchHit {
                file: filename.clone(),
                res_num,
                datetime: post.datetime,
                id: post.id,
                body: post.body,
                urls,
                ref_count,
            });
        }
    }

    // Cumulative cutoff by max_body_chars
    let include_id = params.include_id;
    let omitted_count = dat::apply_cutoff(
        &mut hits,
        params.max_body_chars,
        params.disable_body_limit,
        |h| h.response_chars(include_id),
    );

    let total_hits = hits.len() + omitted_count;
    Ok(SearchPostsResult {
        hits,
        total_hits,
        searched_files,
        omitted_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dat::test_helpers::create_test_dat_dir;

    #[test]
    fn search_basic_keyword() {
        let ctx = create_test_dat_dir();
        let result = search_posts(
            &ctx.dat_dir,
            &SearchPostsParams {
                keywords: vec!["Tool v2\\.5".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_hits, 1);
        assert_eq!(result.hits[0].res_num, 2);
    }

    #[test]
    fn search_multiple_keywords() {
        let ctx = create_test_dat_dir();
        let result = search_posts(
            &ctx.dat_dir,
            &SearchPostsParams {
                keywords: vec!["Tool v2\\.5".into(), "App-X".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_hits, 2);
    }

    #[test]
    fn search_specific_file() {
        let ctx = create_test_dat_dir();
        let result = search_posts(
            &ctx.dat_dir,
            &SearchPostsParams {
                keywords: vec!["Bazqux".into()],
                files: vec!["631".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_hits, 1);
        assert_eq!(result.searched_files.len(), 1);
    }

    #[test]
    fn search_with_range() {
        let ctx = create_test_dat_dir();
        let result = search_posts(
            &ctx.dat_dir,
            &SearchPostsParams {
                keywords: vec!["プラグイン".into()],
                files: vec!["630".into()],
                range: Some("1-2".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_hits, 0); // "プラグイン" is in res 3
    }

    #[test]
    fn search_empty_keywords_error() {
        let ctx = create_test_dat_dir();
        let result = search_posts(
            &ctx.dat_dir,
            &SearchPostsParams {
                keywords: vec![],
                ..Default::default()
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn search_by_id_only() {
        let ctx = create_test_dat_dir();
        // Search by ID only (no keywords)
        let result = search_posts(
            &ctx.dat_dir,
            &SearchPostsParams {
                ids: vec!["test0002".into()],
                files: vec!["630".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_hits, 1);
        assert_eq!(result.hits[0].res_num, 2);
    }

    #[test]
    fn search_by_id_and_keyword() {
        let ctx = create_test_dat_dir();
        // ID + keyword combination
        // test0003 is the "プラグイン" post. Should not match "Tool v2.5"
        let result = search_posts(
            &ctx.dat_dir,
            &SearchPostsParams {
                keywords: vec!["Tool v2\\.5".into()],
                ids: vec!["test0003".into()],
                files: vec!["630".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_hits, 0);
    }

    #[test]
    fn search_by_id_no_match() {
        let ctx = create_test_dat_dir();
        let result = search_posts(
            &ctx.dat_dir,
            &SearchPostsParams {
                ids: vec!["nonexistent_id".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_hits, 0);
    }

    #[test]
    fn search_empty_keywords_and_ids_error() {
        let ctx = create_test_dat_dir();
        let result = search_posts(
            &ctx.dat_dir,
            &SearchPostsParams {
                keywords: vec![],
                ids: vec![],
                ..Default::default()
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn search_max_body_chars_no_limit() {
        let ctx = create_test_dat_dir();
        // max_body_chars=0 (default) means no limit
        let result = search_posts(
            &ctx.dat_dir,
            &SearchPostsParams {
                keywords: vec!["Tool v2\\.5".into(), "プラグイン".into()],
                files: vec!["630".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_hits, 2);
        assert!(result.omitted_count == 0);
    }

    #[test]
    fn search_max_body_chars_truncates() {
        let ctx = create_test_dat_dir();
        // max_body_chars=1 cuts off after the first hit, rest goes to omitted
        let result = search_posts(
            &ctx.dat_dir,
            &SearchPostsParams {
                keywords: vec!["Tool v2\\.5".into(), "プラグイン".into()],
                files: vec!["630".into()],
                max_body_chars: 1,
                ..Default::default()
            },
        )
        .unwrap();
        // total_hits includes omitted count
        assert_eq!(result.total_hits, 2);
        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.omitted_count, 1);
    }

    #[test]
    fn search_max_body_chars_large_enough() {
        let ctx = create_test_dat_dir();
        // Large enough max_body_chars means no omissions
        let result = search_posts(
            &ctx.dat_dir,
            &SearchPostsParams {
                keywords: vec!["Tool v2\\.5".into(), "プラグイン".into()],
                files: vec!["630".into()],
                max_body_chars: 100000,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_hits, 2);
        assert_eq!(result.hits.len(), 2);
        assert!(result.omitted_count == 0);
    }
}
