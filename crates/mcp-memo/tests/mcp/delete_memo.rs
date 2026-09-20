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

use super::*;

#[tokio::test]
async fn delete_memo_should_remove_memo() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;

    ctx.call("set_memo", json!({ "key": "tmp", "content": "data" }))
        .await
        .unwrap();
    let result = ctx
        .call("delete_memo", json!({ "key": "tmp" }))
        .await
        .unwrap();
    assert_eq!(result, "Deleted memo 'tmp'");

    let err = ctx
        .call("get_memo", json!({ "key": "tmp" }))
        .await
        .unwrap_err();
    assert!(err.contains("not found"));
}

#[tokio::test]
async fn delete_memo_should_backup_existing_memo() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;

    ctx.call("set_memo", json!({ "key": "note", "content": "v1" }))
        .await
        .unwrap();
    let result = ctx
        .call("delete_memo", json!({ "key": "note" }))
        .await
        .unwrap();
    assert_eq!(result, "Deleted memo 'note'");

    // Memo should be gone
    let err = ctx
        .call("get_memo", json!({ "key": "note" }))
        .await
        .unwrap_err();
    assert!(err.contains("not found"));

    assert_eq!(
        memo_history(dir.path(), "note"),
        vec![None, Some(b"v1".to_vec()), None]
    );
    assert!(!dir.path().join("backup").exists());
}
