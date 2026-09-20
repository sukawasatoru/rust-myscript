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

use crate::git;
use crate::text::replace_memo_text;
use anyhow::anyhow;
use rust_myscript::prelude::*;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct MemoStore {
    data_dir: PathBuf,
}

impl MemoStore {
    pub(crate) fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    pub(crate) async fn initialize(&self) -> Fallible<()> {
        let data_dir = self.data_dir.clone();
        tokio::task::spawn_blocking(move || {
            fs::create_dir_all(&data_dir)?;
            let _lock = git::lock_directory(&data_dir)?;
            git::ensure_repository(&data_dir)?;
            Ok(())
        })
        .await?
    }

    pub(crate) async fn read_memo(&self, path: &Path) -> std::io::Result<String> {
        tokio::fs::read_to_string(path).await
    }

    pub(crate) async fn write_memo(&self, path: &Path, key: &str, content: &str) -> Fallible<()> {
        self.mutate(path, key, Mutation::Set(content.to_string()))
            .await
    }

    pub(crate) async fn remove_memo(&self, path: &Path, key: &str) -> Fallible<()> {
        self.mutate(path, key, Mutation::Delete).await
    }

    pub(crate) async fn edit_memo(&self, key: &str, old: &str, new: &str) -> Fallible<()> {
        let path = self.key_to_path(key)?;
        self.mutate(
            &path,
            key,
            Mutation::Edit {
                old: old.to_string(),
                new: new.to_string(),
            },
        )
        .await
    }

    async fn mutate(&self, path: &Path, key: &str, mutation: Mutation) -> Fallible<()> {
        let data_dir = self.data_dir.clone();
        let path = path.to_path_buf();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || {
            let action = if matches!(mutation, Mutation::Delete) { "delete" } else { "write" };
            let io_error = |e: anyhow::Error| {
                warn!(?e, %key, "memo mutation failed");
                anyhow!("failed to {action} memo '{key}'")
            };
            let _lock = git::lock_directory(&data_dir).map_err(io_error)?;
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => Some(metadata),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(io_error(e.into())),
            };
            if metadata.as_ref().is_some_and(|m| !m.is_file()) {
                return Err(if matches!(mutation, Mutation::Edit { .. }) {
                    anyhow!("failed to read memo '{key}'")
                } else {
                    anyhow!("failed to {action} memo '{key}'")
                });
            }
            let content = match mutation {
                Mutation::Set(content) => Some(content),
                Mutation::Edit { old, new } => {
                    let content = fs::read_to_string(&path).map_err(|e| {
                        if e.kind() == std::io::ErrorKind::NotFound {
                            anyhow!("memo '{key}' not found")
                        } else {
                            anyhow!("failed to read memo '{key}'")
                        }
                    })?;
                    Some(replace_memo_text(&content, &old, &new, &key)?)
                }
                Mutation::Delete => {
                    if metadata.is_none() {
                        bail!("memo '{key}' not found");
                    }
                    None
                }
            };
            let prepare = || -> Fallible<_> {
                let repo = git::ensure_repository(&data_dir)?;
                let snapshot = git::snapshot(&data_dir)?;
                git::commit_snapshot(&repo, &snapshot, "Recover unrecorded memo changes", &git::signature_now()?, false)?;
                Ok((repo, snapshot))
            };
            let (repo, mut snapshot) = prepare().map_err(|e| anyhow!(
                "memo '{key}' was not changed: could not prepare Git history: {e:#}"
            ))?;
            let name = format!("{key}.txt");
            let message = if let Some(content) = content {
                atomic_write(&data_dir, &path, content.as_bytes()).map_err(io_error)?;
                snapshot.insert(name, content.into_bytes());
                format!("{} memo {key}", if metadata.is_some() { "Update" } else { "Create" })
            } else {
                fs::remove_file(&path).map_err(|e| io_error(e.into()))?;
                snapshot.remove(&name);
                format!("Delete memo {key}")
            };
            let commit = || -> Fallible<()> {
                git::commit_snapshot(&repo, &snapshot, &message, &git::signature_now()?, false)?;
                Ok(())
            };
            commit().map_err(|e| anyhow!(
                "memo '{key}' was changed, but Git history could not be saved; the next write will retry recording it: {e:#}"
            ))
        }).await.map_err(|e| anyhow!("memo operation task failed; check memo state before retrying: {e}"))?
    }

    pub(crate) fn key_to_path(&self, key: &str) -> Fallible<PathBuf> {
        git::validate_key(key)?;
        Ok(self.data_dir.join(format!("{key}.txt")))
    }

    pub(crate) async fn list_memos(&self) -> Fallible<Vec<String>> {
        let mut entries = tokio::fs::read_dir(&self.data_dir).await.inspect_err(|e| {
            warn!(?e, "failed to read data directory");
        })?;
        let mut keys = Vec::new();
        loop {
            match entries.next_entry().await {
                Ok(Some(entry)) => {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    // Check file type to avoid accidentally listing subdirectories (e.g. "backup")
                    // that might have a .txt suffix in the future.
                    let is_file = entry
                        .file_type()
                        .await
                        .map(|t| t.is_file())
                        .unwrap_or(false);
                    if is_file && let Some(key) = name.strip_suffix(".txt") {
                        keys.push(key.to_string());
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    warn!(?e, "failed to read directory entry");
                    return Err(e.into());
                }
            }
        }
        keys.sort();
        Ok(keys)
    }
}

enum Mutation {
    Set(String),
    Edit { old: String, new: String },
    Delete,
}

fn atomic_write(data_dir: &Path, path: &Path, content: &[u8]) -> Fallible<()> {
    let mut temporary = tempfile::Builder::new()
        .prefix(".mcp-memo-write-")
        .tempfile_in(data_dir)?;
    temporary.write_all(content)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|e| e.error)?;
    Ok(())
}
