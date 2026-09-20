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
async fn set_memo_should_return_stored_message() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;

    let result = ctx
        .call("set_memo", json!({ "key": "hello", "content": "world" }))
        .await
        .unwrap();
    assert_eq!(result, "Stored memo 'hello'");
}

#[tokio::test]
async fn set_memo_should_backup_existing_memo() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;

    ctx.call("set_memo", json!({ "key": "note", "content": "v1" }))
        .await
        .unwrap();
    ctx.call("set_memo", json!({ "key": "note", "content": "v2" }))
        .await
        .unwrap();

    // Current memo should be v2
    let result = ctx
        .call("get_memo", json!({ "key": "note" }))
        .await
        .unwrap();
    assert_eq!(result, "v2");

    assert_eq!(
        memo_history(dir.path(), "note"),
        vec![Some(b"v2".to_vec()), Some(b"v1".to_vec()), None]
    );
    assert!(!dir.path().join("backup").exists());
}

#[tokio::test]
async fn updates_should_continue_when_backup_directory_cannot_be_created() {
    for blocked_path in ["backup", "backup/note"] {
        for (tool, args, expected) in [
            (
                "set_memo",
                json!({ "key": "note", "content": "hello rust" }),
                "Stored memo 'note'",
            ),
            (
                "edit_memo",
                json!({ "key": "note", "old": "world", "new": "rust" }),
                "Edited memo 'note'",
            ),
        ] {
            let dir = tempdir().unwrap();
            let ctx = McpTestContext::new(dir.path().to_path_buf()).await;
            let memo_path = dir.path().join("note.txt");
            std::fs::write(&memo_path, "hello world").unwrap();
            let blocked_path = dir.path().join(blocked_path);
            std::fs::create_dir_all(blocked_path.parent().unwrap()).unwrap();
            std::fs::write(&blocked_path, "backup blocker").unwrap();

            let result = ctx.call(tool, args).await.unwrap();
            assert_eq!(result, expected);
            assert_eq!(std::fs::read_to_string(&memo_path).unwrap(), "hello rust");
            assert_eq!(
                ctx.call("get_memo", json!({ "key": "note" }))
                    .await
                    .unwrap(),
                "hello rust"
            );
            assert_eq!(
                std::fs::read_to_string(&blocked_path).unwrap(),
                "backup blocker"
            );
        }
    }
}

#[tokio::test]
async fn set_memo_should_prune_old_backups_beyond_max_count() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;

    // Unlike legacy file backups, Git history must retain all seven versions.
    for i in 1..=7u32 {
        ctx.call(
            "set_memo",
            json!({ "key": "prune", "content": format!("v{i}") }),
        )
        .await
        .unwrap();
    }

    let history = memo_history(dir.path(), "prune");
    assert_eq!(history.len(), 8);
    for (index, version) in (1..=7).rev().enumerate() {
        assert_eq!(history[index], Some(format!("v{version}").into_bytes()));
    }
    assert_eq!(history[7], None);
    assert!(!dir.path().join("backup").exists());
}
