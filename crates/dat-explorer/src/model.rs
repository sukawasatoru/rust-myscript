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

impl DatPost {
    /// Returns the estimated character count for response fields.
    /// Only counts fields actually included in the response.
    pub fn response_chars(&self, include_name: bool) -> usize {
        let name_chars = if include_name {
            self.name.chars().count()
        } else {
            0
        };
        name_chars
            + self.datetime.chars().count()
            + self.id.chars().count()
            + self.body.chars().count()
            + self.title.as_deref().map_or(0, |t| t.chars().count())
    }
}

/// Metadata for a dat file.
#[derive(Debug, Clone)]
pub struct DatFileInfo {
    pub filename: String,
    pub thread_num: u32,
    pub thread_id: String,
    pub total_lines: usize,
    pub thread_title: String,
    pub date_range: String,
}
