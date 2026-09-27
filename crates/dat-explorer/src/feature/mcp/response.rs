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

//! MCP-specific response budgeting; never applied to direct application access.

use crate::model::{DatPost, query::SearchHit};

const MAX_BODY_CHARS_LIMIT: usize = 50000;
const ROW_OVERHEAD: usize = 30;

fn effective_max_body_chars(value: usize, disable_limit: bool) -> Option<usize> {
    if disable_limit {
        if value == 0 { None } else { Some(value) }
    } else if value == 0 || value > MAX_BODY_CHARS_LIMIT {
        Some(MAX_BODY_CHARS_LIMIT)
    } else {
        Some(value)
    }
}

/// Includes the item that crosses the limit; only subsequent items are omitted.
pub fn apply_cutoff<T>(
    items: &mut Vec<T>,
    max_body_chars: usize,
    disable_body_limit: bool,
    char_count: impl Fn(&T) -> usize,
) -> usize {
    let effective_limit = match effective_max_body_chars(max_body_chars, disable_body_limit) {
        Some(limit) => limit,
        None => return 0,
    };
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

pub fn post_chars(post: &DatPost, include_name: bool, include_id: bool) -> usize {
    let name_chars = if include_name {
        post.name.chars().count()
    } else {
        0
    };
    let id_chars = if include_id {
        post.id.chars().count()
    } else {
        0
    };
    name_chars + post.datetime.chars().count() + id_chars + post.body.chars().count()
}

pub fn hit_chars(hit: &SearchHit, include_id: bool) -> usize {
    let id_chars = if include_id {
        hit.id.chars().count()
    } else {
        0
    };
    hit.file.chars().count()
        + hit.datetime.chars().count()
        + id_chars
        + hit.body.chars().count()
        + hit.urls.iter().map(|u| u.chars().count()).sum::<usize>()
}
