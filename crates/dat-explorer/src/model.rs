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

pub mod fetch;
pub mod query;
pub mod urls;
pub mod viewer;

/// A complete parsed snapshot. Source line positions are retained for range queries,
/// including malformed lines and the terminal "Over 1000 Thread" line.
#[derive(Debug, Clone)]
pub struct LoadedThread {
    pub file_info: DatFileInfo,
    pub posts: Vec<DatPost>,
    pub source_line_count: usize,
    pub ref_counts: std::collections::HashMap<usize, usize>,
}

/// Downloaded UTF-8 dat ready for storage, not a lossy reconstruction from display posts.
/// Its text is kept opaque to callers outside the crate.
pub struct DownloadedDat {
    text: String,
}

impl DownloadedDat {
    pub(crate) fn from_text(text: String) -> Self {
        Self { text }
    }

    pub fn res_count(&self) -> usize {
        self.text.lines().filter(|line| !line.is_empty()).count()
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }
}

/// A single parsed post from a dat file.
#[derive(Debug, Clone)]
pub struct DatPost {
    pub res_num: usize,
    pub name: String,
    pub mail: String,
    pub datetime: String,
    pub id: String,
    pub body: String,
    pub title: Option<String>,
}

/// Metadata derived solely from dat contents, without a storage identity.
#[derive(Debug, Clone)]
pub struct DatContentInfo {
    pub total_lines: usize,
    pub thread_title: String,
    pub date_range: String,
}

/// Content metadata combined with local storage identity.
#[derive(Debug, Clone)]
pub struct DatFileInfo {
    pub filename: String,
    pub thread_num: u32,
    pub thread_id: String,
    pub total_lines: usize,
    pub thread_title: String,
    pub date_range: String,
}
