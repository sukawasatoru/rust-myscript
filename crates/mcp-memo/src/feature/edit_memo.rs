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

pub(crate) async fn edit_memo(store: &MemoStore, key: &str, old: &str, new: &str) -> Fallible<()> {
    // Reject empty old before key validation and file I/O to preserve error precedence.
    if old.is_empty() {
        bail!("old must not be empty");
    }
    store.edit_memo(key, old, new).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn edit_memo_should_validate_before_mutating() {
        let dir = tempdir().unwrap();
        let store = MemoStore::new(dir.path().to_path_buf());
        assert_eq!(
            edit_memo(&store, "invalid/key", "", "new")
                .await
                .unwrap_err()
                .to_string(),
            "old must not be empty"
        );
        store.initialize().await.unwrap();
        super::super::set_memo(&store, "doc", "ああああ")
            .await
            .unwrap();
        assert_eq!(
            edit_memo(&store, "doc", "あああ", "new")
                .await
                .unwrap_err()
                .to_string(),
            "old text occurs multiple times in memo 'doc'"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("doc.txt")).unwrap(),
            "ああああ"
        );
        assert!(!dir.path().join("backup").exists());
        edit_memo(&store, "doc", "ああああ", "new").await.unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("doc.txt")).unwrap(),
            "new"
        );
        assert!(!dir.path().join("backup").exists());
        let repo = crate::git::open_repository(dir.path()).unwrap().unwrap();
        let parent = repo
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .parent(0)
            .unwrap();
        let tree = parent.tree().unwrap();
        let blob = repo
            .find_blob(tree.get_name("doc.txt").unwrap().id())
            .unwrap();
        assert_eq!(blob.content(), "ああああ".as_bytes());
    }
}
