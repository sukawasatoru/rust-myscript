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

use crate::store::MemoStore;
use rust_myscript::prelude::*;

pub(crate) async fn list_memos(store: &MemoStore) -> Fallible<Vec<String>> {
    let mut keys = store
        .list_memos()
        .await
        .context("failed to read memo list")?;
    keys.sort();
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn list_memos_should_return_sorted_memo_files_only() {
        let dir = tempdir().unwrap();
        let store = MemoStore::new(dir.path().to_path_buf());
        assert!(list_memos(&store).await.unwrap().is_empty());
        for name in ["z.txt", "a.txt", "ignored.md"] {
            std::fs::write(dir.path().join(name), "content").unwrap();
        }
        std::fs::create_dir(dir.path().join("directory.txt")).unwrap();
        std::fs::create_dir(dir.path().join("backup")).unwrap();
        assert_eq!(list_memos(&store).await.unwrap(), ["a", "z"]);
    }

    #[tokio::test]
    async fn list_memos_should_report_directory_errors() {
        let dir = tempdir().unwrap();
        let store = MemoStore::new(dir.path().join("missing"));
        assert_eq!(
            list_memos(&store).await.unwrap_err().to_string(),
            "failed to read memo list"
        );
    }
}
