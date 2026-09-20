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

pub(crate) async fn set_memo(store: &MemoStore, key: &str, content: &str) -> Fallible<()> {
    let path = store.key_to_path(key)?;
    store.write_memo(&path, key, content).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn set_memo_should_validate_before_writing() {
        let dir = tempdir().unwrap();
        let store = MemoStore::new(dir.path().to_path_buf());
        assert_eq!(
            set_memo(&store, "", "content")
                .await
                .unwrap_err()
                .to_string(),
            "key must not be empty"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        set_memo(&store, "doc", "content").await.unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("doc.txt")).unwrap(),
            "content"
        );
    }

    #[tokio::test]
    async fn set_memo_should_report_write_errors() {
        let dir = tempdir().unwrap();
        let store = MemoStore::new(dir.path().join("missing"));
        assert_eq!(
            set_memo(&store, "doc", "content")
                .await
                .unwrap_err()
                .to_string(),
            "failed to write memo 'doc'"
        );
    }
}
