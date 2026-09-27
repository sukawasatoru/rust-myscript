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

pub struct FetchDatParams {
    /// A dat or read.cgi URL. Direct dat is tried first, with HTML fallback on 404.
    pub url: String,
    /// Absolute path, or a path relative to the service's dat directory.
    pub save_path: String,
}

pub struct FetchDatResult {
    pub save_path: String,
    /// Number of non-empty lines in the downloaded dat.
    pub res_count: usize,
    /// None for a new file; increases saturate at zero.
    pub added_res_count: Option<usize>,
}

pub struct FetchSubjectParams {
    /// HTTP(S) URL; userinfo is allowed, fragment stripped, query preserved.
    pub url: String,
    /// Case-insensitive substring after NCR restoration. None/empty matches all.
    pub title_contains: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubjectThread {
    /// Kept as a string to preserve leading zeros.
    pub thread_id: String,
    /// NCR-restored title, with leading spaces preserved.
    pub title: String,
    pub res_count: u64,
}

#[derive(Debug)]
pub struct FetchSubjectResult {
    /// In subject.txt order, including duplicates.
    pub threads: Vec<SubjectThread>,
}
