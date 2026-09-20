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

pub(crate) async fn delete_memo(store: &MemoStore, key: &str) -> Fallible<()> {
    let path = store.key_to_path(key)?;
    store.remove_memo(&path, key).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn delete_memo_should_report_missing_and_failed_deletes() {
        let dir = tempdir().unwrap();
        let store = MemoStore::new(dir.path().to_path_buf());
        assert_eq!(
            delete_memo(&store, "missing")
                .await
                .unwrap_err()
                .to_string(),
            "memo 'missing' not found"
        );
        std::fs::create_dir(dir.path().join("doc.txt")).unwrap();
        std::fs::write(dir.path().join("backup"), "block backups").unwrap();
        assert_eq!(
            delete_memo(&store, "doc").await.unwrap_err().to_string(),
            "failed to delete memo 'doc'"
        );
        assert!(dir.path().join("doc.txt").is_dir());
    }
}
