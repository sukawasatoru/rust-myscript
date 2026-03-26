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
use crate::model::{DatFileInfo, DatPost};
use rust_myscript::prelude::*;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

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
    /// When true, the safety cap (MAX_BODY_CHARS_LIMIT) is not applied.
    pub disable_body_limit: bool,
}

pub struct ReadPostsResult {
    pub posts: Vec<DatPost>,
    pub file_info: DatFileInfo,
    /// Post number -> reference count (>>N anchor aggregation)
    pub ref_counts: HashMap<usize, usize>,
    /// Number of posts omitted due to max_body_chars exceeded
    pub omitted_count: usize,
}

pub fn read_posts(dat_dir: &Path, params: &ReadPostsParams) -> Fallible<ReadPostsResult> {
    let path = dat::resolve_dat_file(dat_dir, &params.file)?;
    let lines = dat::read_lines(&path)?;
    let file_info = dat::build_file_info_from_lines(&path, &lines)?;
    let total = lines.len();

    let mut posts = Vec::new();

    if !params.res_nums.is_empty() {
        // Retrieve only the specified post numbers (sorted)
        let target: BTreeSet<usize> = params.res_nums.iter().copied().collect();
        for &res_num in &target {
            let i = res_num - 1;
            if i >= lines.len() {
                continue;
            }
            if let Some(post) = dat::parse_dat_line(&lines[i], res_num) {
                posts.push(post);
            }
        }
    } else {
        // Retrieve posts within the specified range
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
            if let Some(post) = dat::parse_dat_line(line, res_num) {
                posts.push(post);
            }
        }
    }

    let ref_counts = dat::count_references(&lines);

    // Cumulative cutoff by max_body_chars
    let include_name = params.include_name;
    let omitted_count = dat::apply_cutoff(
        &mut posts,
        params.max_body_chars,
        params.disable_body_limit,
        |p| p.response_chars(include_name),
    );

    Ok(ReadPostsResult {
        posts,
        file_info,
        ref_counts,
        omitted_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dat::test_helpers::create_test_dat_dir;

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
