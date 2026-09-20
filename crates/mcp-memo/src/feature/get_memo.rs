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

pub(crate) async fn get_memo(store: &MemoStore, key: &str) -> Fallible<String> {
    let path = store.key_to_path(key)?;
    match store.read_memo(&path).await {
        Ok(content) => Ok(content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!("memo '{key}' not found")
        }
        Err(e) => {
            warn!(?e, %key, "failed to read memo");
            bail!("failed to read memo '{key}'")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn get_memo_should_read_content_and_report_read_errors() {
        let dir = tempdir().unwrap();
        let store = MemoStore::new(dir.path().to_path_buf());
        std::fs::write(dir.path().join("doc.txt"), "日本語のメモ").unwrap();
        assert_eq!(get_memo(&store, "doc").await.unwrap(), "日本語のメモ");
        assert_eq!(
            get_memo(&store, "missing").await.unwrap_err().to_string(),
            "memo 'missing' not found"
        );
        std::fs::create_dir(dir.path().join("unreadable.txt")).unwrap();
        assert_eq!(
            get_memo(&store, "unreadable")
                .await
                .unwrap_err()
                .to_string(),
            "failed to read memo 'unreadable'"
        );
    }
}
