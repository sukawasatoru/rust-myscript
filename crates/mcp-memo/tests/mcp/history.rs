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
async fn commit_failure_keeps_content_and_recovers_before_next_update() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;
    ctx.call("set_memo", json!({"key": "doc", "content": "v1"}))
        .await
        .unwrap();
    let index_lock = dir.path().join(".git/index.lock");
    std::fs::write(&index_lock, "block index writes").unwrap();
    let err = ctx
        .call("set_memo", json!({"key": "doc", "content": "v2"}))
        .await
        .unwrap_err();
    assert!(
        err.contains("was changed, but Git history could not be saved"),
        "{err}"
    );
    assert_eq!(
        ctx.call("get_memo", json!({"key": "doc"})).await.unwrap(),
        "v2"
    );
    assert_eq!(
        memo_history(dir.path(), "doc"),
        vec![Some(b"v1".to_vec()), None]
    );

    let err = ctx
        .call("set_memo", json!({"key": "doc", "content": "v3"}))
        .await
        .unwrap_err();
    assert!(err.contains("was not changed"), "{err}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("doc.txt")).unwrap(),
        "v2"
    );
    std::fs::remove_file(index_lock).unwrap();
    ctx.call("set_memo", json!({"key": "doc", "content": "v3"}))
        .await
        .unwrap();
    assert_eq!(
        memo_history(dir.path(), "doc"),
        vec![
            Some(b"v3".to_vec()),
            Some(b"v2".to_vec()),
            Some(b"v1".to_vec()),
            None
        ]
    );
}

#[tokio::test]
async fn failed_deletion_commit_is_recovered_after_restart() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;
    ctx.call("set_memo", json!({"key": "doc", "content": "old"}))
        .await
        .unwrap();
    let index_lock = dir.path().join(".git/index.lock");
    std::fs::write(&index_lock, "block index writes").unwrap();
    let err = ctx
        .call("delete_memo", json!({"key": "doc"}))
        .await
        .unwrap_err();
    assert!(
        err.contains("was changed, but Git history could not be saved"),
        "{err}"
    );
    assert!(!dir.path().join("doc.txt").exists());
    drop(ctx);
    std::fs::remove_file(index_lock).unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;
    ctx.call("set_memo", json!({"key": "doc", "content": "new"}))
        .await
        .unwrap();
    assert_eq!(
        memo_history(dir.path(), "doc"),
        vec![Some(b"new".to_vec()), None, Some(b"old".to_vec()), None]
    );
}

#[tokio::test]
async fn same_content_does_not_create_another_commit() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;
    for _ in 0..2 {
        ctx.call("set_memo", json!({"key": "doc", "content": "same"}))
            .await
            .unwrap();
    }
    assert_eq!(
        memo_history(dir.path(), "doc"),
        vec![Some(b"same".to_vec()), None]
    );
}

#[tokio::test]
async fn separate_servers_serialize_read_modify_write() {
    let dir = tempdir().unwrap();
    let first = McpTestContext::new(dir.path().to_path_buf()).await;
    let second = McpTestContext::new(dir.path().to_path_buf()).await;
    first
        .call("set_memo", json!({"key": "doc", "content": "a b"}))
        .await
        .unwrap();
    let (one, two) = tokio::join!(
        first.call("edit_memo", json!({"key": "doc", "old": "a", "new": "A"})),
        second.call("edit_memo", json!({"key": "doc", "old": "b", "new": "B"})),
    );
    one.unwrap();
    two.unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("doc.txt")).unwrap(),
        "A B"
    );
    assert_eq!(memo_history(dir.path(), "doc").len(), 4);
}

#[tokio::test]
async fn commits_have_explicit_identity_and_current_dates() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;
    let start = chrono::Utc::now().timestamp();
    ctx.call("set_memo", json!({"key": "doc", "content": "created now"}))
        .await
        .unwrap();
    let repo = git2::Repository::open(dir.path()).unwrap();
    let commit = repo.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(commit.author().name().unwrap(), "mcp-memo");
    assert_eq!(commit.author().email().unwrap(), "mcp-memo@localhost");
    assert!(commit.time().seconds() >= start);
    assert!(commit.time().seconds() <= chrono::Utc::now().timestamp());
    assert_eq!(commit.author().when(), commit.committer().when());
}
