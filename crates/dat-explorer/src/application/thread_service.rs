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

//! Shared operations without MCP schemas or response budgets.

use crate::data::{dat_file, five_ch};
use crate::model::DownloadedDat;
use crate::model::fetch::{FetchDatParams, FetchDatResult, FetchSubjectParams, FetchSubjectResult};
use crate::model::query::{
    PostMatcher, ReadPostsQuery, ReadPostsResult, SearchHit, SearchPostsQuery, SearchPostsResult,
    resolve_range,
};
use crate::model::urls::extract_urls;
use rust_myscript::prelude::*;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

pub struct ThreadService {
    dat_dir: PathBuf,
    subject_client: reqwest::Client,
}

impl ThreadService {
    pub fn new(dat_dir: PathBuf, subject_client: reqwest::Client) -> Self {
        Self {
            dat_dir,
            subject_client,
        }
    }

    pub fn read_posts(&self, query: &ReadPostsQuery) -> Fallible<ReadPostsResult> {
        let path = dat_file::resolve_dat_file(&self.dat_dir, &query.file)?;
        let thread = dat_file::load_thread(&path)?;
        let file_info = thread.file_info;
        let total = thread.source_line_count;
        let mut posts = thread.posts;
        if !query.res_nums.is_empty() {
            let target: BTreeSet<usize> = query.res_nums.iter().copied().collect();
            posts.retain(|post| target.contains(&post.res_num));
        } else {
            let (start, end) = if let Some(ref range) = query.range {
                resolve_range(range, total)?
            } else {
                (1, total)
            };
            posts.retain(|post| start <= post.res_num && post.res_num <= end);
        }
        let ref_counts = thread.ref_counts;
        let urls = if query.include_urls {
            posts
                .iter()
                .map(|p| (p.res_num, extract_urls(&p.body)))
                .collect()
        } else {
            HashMap::new()
        };
        Ok(ReadPostsResult {
            posts,
            file_info,
            ref_counts,
            urls,
        })
    }

    pub fn search_posts(&self, query: &SearchPostsQuery) -> Fallible<SearchPostsResult> {
        let matcher = PostMatcher::new(query)?;
        let paths = dat_file::resolve_files(&self.dat_dir, &query.files)?;
        let mut hits = Vec::new();
        let mut searched_files = Vec::new();
        for path in &paths {
            let thread = dat_file::load_thread(path)?;
            let filename = thread.file_info.filename;
            searched_files.push(filename.clone());
            let ref_counts = thread.ref_counts;
            let (start, end) = if let Some(ref range) = query.range {
                resolve_range(range, thread.source_line_count)?
            } else {
                (1, thread.source_line_count)
            };
            for post in thread.posts {
                let res_num = post.res_num;
                if res_num < start || res_num > end {
                    continue;
                }
                if !matcher.matches(&post) {
                    continue;
                }
                let urls = extract_urls(&post.body);
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
        Ok(SearchPostsResult {
            hits,
            searched_files,
        })
    }

    pub async fn fetch_dat(&self, params: &FetchDatParams) -> Fallible<FetchDatResult> {
        let path = Path::new(&params.save_path);
        let save_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.dat_dir.join(path)
        };
        let existing_res_count = dat_file::count_lines_if_exists(&save_path);
        let dat = five_ch::dat::download_dat(&params.url).await?;
        save_downloaded_dat(&save_path, &dat, existing_res_count).await
    }

    pub async fn fetch_subject(&self, params: &FetchSubjectParams) -> Fallible<FetchSubjectResult> {
        five_ch::subject::fetch_subject(&self.subject_client, params).await
    }
}

/// Save the downloaded snapshot and report the change against its previous version.
async fn save_downloaded_dat(
    save_path: &Path,
    dat: &DownloadedDat,
    existing_res_count: Option<usize>,
) -> Fallible<FetchDatResult> {
    let res_count = dat.res_count();
    dat_file::save_dat(save_path, dat).await?;
    Ok(FetchDatResult {
        save_path: save_path.to_string_lossy().into_owned(),
        res_count,
        added_res_count: existing_res_count.map(|prev| res_count.saturating_sub(prev)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_use_source_positions_even_when_some_lines_are_not_posts() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("board_700_123.dat"),
            "名無し<><>2026/01/01 ID:a<>本文1<>タイトル\ninvalid\n名無し<><>2026/01/02 ID:b<>本文3<>\nOver 1000 Thread\n"
        ).unwrap();
        let service = ThreadService::new(dir.path().to_path_buf(), reqwest::Client::new());
        let read = service
            .read_posts(&ReadPostsQuery {
                file: "700".into(),
                range: Some("-2".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            read.posts.iter().map(|p| p.res_num).collect::<Vec<_>>(),
            [3]
        );
        let search = service
            .search_posts(&SearchPostsQuery {
                files: vec!["700".into()],
                keywords: vec!["本文".into()],
                range: Some("-2".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            search.hits.iter().map(|p| p.res_num).collect::<Vec<_>>(),
            [3]
        );
        let read = service
            .read_posts(&ReadPostsQuery {
                file: "700".into(),
                range: Some("1-1".into()),
                res_nums: vec![3, 2, 3],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            read.posts.iter().map(|p| p.res_num).collect::<Vec<_>>(),
            [3]
        );
    }

    #[tokio::test]
    async fn save_download_tracks_new_unchanged_grown_and_shorter_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("nested/thread.dat");
        for (text, expected_count, expected_added) in [
            ("一行目\n\n二行目\n", 2, None),
            ("一行目\n二行目\n", 2, Some(0)),
            ("一行目\n二行目\n三行目", 3, Some(1)),
            ("一行目", 1, Some(0)),
        ] {
            let previous = dat_file::count_lines_if_exists(&path);
            let dat = DownloadedDat::from_text(text.to_owned());
            let result = save_downloaded_dat(&path, &dat, previous).await.unwrap();
            assert_eq!(result.save_path, path.to_string_lossy());
            assert_eq!(result.res_count, expected_count);
            assert_eq!(result.added_res_count, expected_added);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        }
    }

    #[tokio::test]
    async fn failed_download_does_not_overwrite_existing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("thread.dat");
        std::fs::write(&path, "existing dat").unwrap();
        let service = ThreadService::new(dir.path().to_path_buf(), reqwest::Client::new());
        assert!(
            service
                .fetch_dat(&FetchDatParams {
                    url: "https://example.com/invalid".into(),
                    save_path: "thread.dat".into(),
                })
                .await
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), "existing dat");
    }

    #[test]
    fn shared_reads_and_searches_have_no_mcp_character_limit() {
        let dir = tempfile::TempDir::new().unwrap();
        let body = "本文".repeat(30000);
        let text = format!(
            "名無し<><>2026/01/01 ID:a<>{body}<>タイトル\n名無し<><>2026/01/02 ID:b<>本文その2<>"
        );
        std::fs::write(dir.path().join("board_700_123.dat"), text).unwrap();
        let service = ThreadService::new(dir.path().to_path_buf(), reqwest::Client::new());
        let read = service
            .read_posts(&ReadPostsQuery {
                file: "700".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(read.posts.len(), 2);
        assert_eq!(read.posts[0].body, body);
        let search = service
            .search_posts(&SearchPostsQuery {
                keywords: vec!["本文".into()],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(search.hits.len(), 2);
        assert_eq!(search.hits[0].body, body);
    }
}
