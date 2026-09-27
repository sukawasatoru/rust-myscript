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

use crate::application::thread_service::ThreadService;
use crate::feature::mcp::response;
use crate::model::query::{SearchHit, SearchPostsQuery};
use rust_myscript::prelude::*;

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

pub struct SearchPostsResult {
    pub hits: Vec<SearchHit>,
    pub total_hits: usize,
    pub searched_files: Vec<String>,
    /// Number of hits omitted due to max_body_chars exceeded
    pub omitted_count: usize,
}

pub fn search_posts(
    service: &ThreadService,
    params: &SearchPostsParams,
) -> Fallible<SearchPostsResult> {
    let result = service.search_posts(&SearchPostsQuery {
        keywords: params.keywords.clone(),
        files: params.files.clone(),
        range: params.range.clone(),
        ids: params.ids.clone(),
    })?;
    let mut hits = result.hits;
    let searched_files = result.searched_files;

    // Cumulative cutoff by max_body_chars
    let include_id = params.include_id;
    let omitted_count = response::apply_cutoff(
        &mut hits,
        params.max_body_chars,
        params.disable_body_limit,
        |h| response::hit_chars(h, include_id),
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
    use crate::data::dat_file::test_helpers::create_test_dat_dir;
    use std::path::Path;

    fn search_posts(dat_dir: &Path, params: &SearchPostsParams) -> Fallible<SearchPostsResult> {
        let service = ThreadService::new(dat_dir.to_path_buf(), reqwest::Client::new());
        super::search_posts(&service, params)
    }

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
