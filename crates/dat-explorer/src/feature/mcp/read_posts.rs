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
use crate::model::query::ReadPostsQuery;
use crate::model::{DatFileInfo, DatPost};
use rust_myscript::prelude::*;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct ReadPostsParams {
    pub file: String,
    pub range: Option<String>,
    /// Specific post numbers to retrieve. Overrides range when specified.
    pub res_nums: Vec<usize>,
    /// Cumulative character limit. Includes the post that exceeds the limit,
    /// remaining count returned as omitted_count. 0 = no limit.
    pub max_body_chars: usize,
    /// Whether the name field is included in the response.
    /// Affects cutoff calculation: excluded name chars are not counted.
    pub include_name: bool,
    /// Whether the id field is included in the response.
    /// Affects cutoff calculation: excluded id chars are not counted.
    pub include_id: bool,
    /// Whether to extract and include URLs from post bodies.
    pub include_urls: bool,
    /// When true, the safety cap (MAX_BODY_CHARS_LIMIT) is not applied.
    pub disable_body_limit: bool,
}

pub struct ReadPostsResult {
    pub posts: Vec<DatPost>,
    pub file_info: DatFileInfo,
    /// Post number -> reference count (>>N anchor aggregation)
    pub ref_counts: HashMap<usize, usize>,
    /// Post number -> extracted URLs (only populated when include_urls is true)
    pub urls: HashMap<usize, Vec<String>>,
    /// Number of posts omitted due to max_body_chars exceeded
    pub omitted_count: usize,
}

pub fn read_posts(service: &ThreadService, params: &ReadPostsParams) -> Fallible<ReadPostsResult> {
    let result = service.read_posts(&ReadPostsQuery {
        file: params.file.clone(),
        range: params.range.clone(),
        res_nums: params.res_nums.clone(),
        include_urls: params.include_urls,
    })?;
    let mut posts = result.posts;
    let file_info = result.file_info;
    let ref_counts = result.ref_counts;
    let urls = result.urls;

    // Cumulative cutoff by max_body_chars
    let include_name = params.include_name;
    let include_id = params.include_id;
    let omitted_count = response::apply_cutoff(
        &mut posts,
        params.max_body_chars,
        params.disable_body_limit,
        |p| {
            let url_chars = urls
                .get(&p.res_num)
                .map_or(0, |u| u.iter().map(|s| s.chars().count()).sum());
            response::post_chars(p, include_name, include_id) + url_chars
        },
    );

    // Remove URLs for omitted posts
    let retained: HashSet<usize> = posts.iter().map(|p| p.res_num).collect();
    let urls: HashMap<usize, Vec<String>> = urls
        .into_iter()
        .filter(|(k, _)| retained.contains(k))
        .collect();

    Ok(ReadPostsResult {
        posts,
        file_info,
        ref_counts,
        urls,
        omitted_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::dat_file::test_helpers::create_test_dat_dir;
    use std::path::Path;

    fn read_posts(dat_dir: &Path, params: &ReadPostsParams) -> Fallible<ReadPostsResult> {
        let service = ThreadService::new(dat_dir.to_path_buf(), reqwest::Client::new());
        super::read_posts(&service, params)
    }

    #[test]
    fn read_all_posts() {
        let ctx = create_test_dat_dir();
        let result = read_posts(
            &ctx.dat_dir,
            &ReadPostsParams {
                file: "630".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.posts.len(), 5);
        assert_eq!(result.file_info.thread_num, 630);
    }

    #[test]
    fn read_with_range() {
        let ctx = create_test_dat_dir();
        let result = read_posts(
            &ctx.dat_dir,
            &ReadPostsParams {
                file: "630".into(),
                range: Some("2-4".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.posts.len(), 3);
        assert_eq!(result.posts[0].res_num, 2);
        assert_eq!(result.posts[2].res_num, 4);
    }

    #[test]
    fn read_with_last_n() {
        let ctx = create_test_dat_dir();
        let result = read_posts(
            &ctx.dat_dir,
            &ReadPostsParams {
                file: "630".into(),
                range: Some("-2".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.posts.len(), 2);
        assert_eq!(result.posts[0].res_num, 4);
        assert_eq!(result.posts[1].res_num, 5);
    }

    #[test]
    fn read_first_post_has_title() {
        let ctx = create_test_dat_dir();
        let result = read_posts(
            &ctx.dat_dir,
            &ReadPostsParams {
                file: "630".into(),
                range: Some("1-1".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.posts[0].title.as_deref(), Some("テストスレッド★630"));
    }

    #[test]
    fn read_cleans_html() {
        let ctx = create_test_dat_dir();
        let result = read_posts(
            &ctx.dat_dir,
            &ReadPostsParams {
                file: "630".into(),
                range: Some("2-2".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let body = &result.posts[0].body;
        assert!(!body.contains("<br>"));
        assert!(body.contains('\n'));
        assert!(body.contains(">>1"));
    }

    #[test]
    fn read_by_full_filename() {
        let ctx = create_test_dat_dir();
        let result = read_posts(
            &ctx.dat_dir,
            &ReadPostsParams {
                file: "board_631_1773831807.dat".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.posts.len(), 3);
        assert_eq!(result.file_info.thread_num, 631);
    }

    #[test]
    fn read_with_res_nums() {
        let ctx = create_test_dat_dir();
        let result = read_posts(
            &ctx.dat_dir,
            &ReadPostsParams {
                file: "630".into(),
                res_nums: vec![1, 3, 5],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.posts.len(), 3);
        assert_eq!(result.posts[0].res_num, 1);
        assert_eq!(result.posts[1].res_num, 3);
        assert_eq!(result.posts[2].res_num, 5);
    }

    #[test]
    fn read_with_res_nums_ignores_range() {
        let ctx = create_test_dat_dir();
        // range is ignored when res_nums is specified
        let result = read_posts(
            &ctx.dat_dir,
            &ReadPostsParams {
                file: "630".into(),
                range: Some("1-1".into()),
                res_nums: vec![2, 4],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.posts.len(), 2);
        assert_eq!(result.posts[0].res_num, 2);
        assert_eq!(result.posts[1].res_num, 4);
    }

    #[test]
    fn read_with_res_nums_out_of_range() {
        let ctx = create_test_dat_dir();
        // Non-existent post numbers are ignored
        let result = read_posts(
            &ctx.dat_dir,
            &ReadPostsParams {
                file: "630".into(),
                res_nums: vec![1, 999],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.posts.len(), 1);
        assert_eq!(result.posts[0].res_num, 1);
    }

    #[test]
    fn read_include_urls() {
        let ctx = create_test_dat_dir();
        let result = read_posts(
            &ctx.dat_dir,
            &ReadPostsParams {
                file: "630".into(),
                range: Some("1-1".into()),
                include_urls: true,
                ..Default::default()
            },
        )
        .unwrap();
        let urls = result.urls.get(&1).unwrap();
        assert_eq!(urls.len(), 1);
        assert!(urls[0].contains("example.com"));
    }

    #[test]
    fn read_include_urls_false() {
        let ctx = create_test_dat_dir();
        let result = read_posts(
            &ctx.dat_dir,
            &ReadPostsParams {
                file: "630".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(result.urls.is_empty());
    }

    #[test]
    fn read_max_body_chars_cutoff() {
        let ctx = create_test_dat_dir();
        // max_body_chars=1 cuts off after the first post (soft overflow)
        let result = read_posts(
            &ctx.dat_dir,
            &ReadPostsParams {
                file: "630".into(),
                max_body_chars: 1,
                ..Default::default()
            },
        )
        .unwrap();
        // First post is included with its full body
        assert_eq!(result.posts.len(), 1);
        assert_eq!(result.posts[0].res_num, 1);
        assert!(result.posts[0].body.chars().count() > 1);
        // Remaining 4 posts are omitted
        assert_eq!(result.omitted_count, 4);
    }

    #[test]
    fn read_max_body_chars_large_enough() {
        let ctx = create_test_dat_dir();
        // Large enough max_body_chars means no omissions
        let result = read_posts(
            &ctx.dat_dir,
            &ReadPostsParams {
                file: "630".into(),
                max_body_chars: 100000,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.posts.len(), 5);
        assert_eq!(result.omitted_count, 0);
    }
}
