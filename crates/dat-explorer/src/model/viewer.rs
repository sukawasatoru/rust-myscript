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

//! Values shared by the local and MCP reading paths.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadEntry {
    pub file: String,
    pub title: String,
    pub post_count: usize,
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadSnapshot {
    pub title: String,
    pub posts: Vec<ViewerPost>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewerPost {
    pub number: usize,
    pub name: String,
    pub datetime: String,
    pub id: String,
    pub body: String,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    #[default]
    Direct,
    Mcp,
}

impl BackendKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Direct => "Direct",
            Self::Mcp => "MCP",
        }
    }
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ViewerConfig {
    pub backend: BackendKind,
}

/// UTF-8 byte offset in the formatted post, stable across terminal width changes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct ReadingPosition {
    pub post_number: usize,
    pub text_offset: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DirectoryState {
    pub path: PathBuf,
    pub selected_file: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ThreadState {
    pub directory: PathBuf,
    pub file: String,
    pub position: ReadingPosition,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ViewerState {
    pub version: u32,
    pub directories: Vec<DirectoryState>,
    pub threads: Vec<ThreadState>,
}

impl Default for ViewerState {
    fn default() -> Self {
        Self {
            version: 1,
            directories: Vec::new(),
            threads: Vec::new(),
        }
    }
}
