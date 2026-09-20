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
async fn edit_memo_should_replace_single_occurrence() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;

    ctx.call(
        "set_memo",
        json!({ "key": "doc", "content": "hello world" }),
    )
    .await
    .unwrap();
    let result = ctx
        .call(
            "edit_memo",
            json!({ "key": "doc", "old": "world", "new": "rust" }),
        )
        .await
        .unwrap();
    assert_eq!(result, "Edited memo 'doc'");

    let content = ctx.call("get_memo", json!({ "key": "doc" })).await.unwrap();
    assert_eq!(content, "hello rust");
}

#[tokio::test]
async fn edit_memo_should_fail_when_old_occurs_multiple_times() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;

    ctx.call("set_memo", json!({ "key": "doc", "content": "ab ab ab" }))
        .await
        .unwrap();
    let err = ctx
        .call(
            "edit_memo",
            json!({ "key": "doc", "old": "ab", "new": "X" }),
        )
        .await
        .unwrap_err();
    assert!(err.contains("multiple times"));
}

#[tokio::test]
async fn edit_memo_should_fail_when_memo_not_found() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;

    let err = ctx
        .call(
            "edit_memo",
            json!({ "key": "missing", "old": "x", "new": "y" }),
        )
        .await
        .unwrap_err();
    assert!(err.contains("not found"));
}

#[tokio::test]
async fn edit_memo_should_fail_when_old_not_found() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;

    ctx.call("set_memo", json!({ "key": "doc", "content": "hello" }))
        .await
        .unwrap();
    let err = ctx
        .call(
            "edit_memo",
            json!({ "key": "doc", "old": "world", "new": "rust" }),
        )
        .await
        .unwrap_err();
    assert!(err.contains("not found"));
}

#[tokio::test]
async fn edit_memo_should_backup_before_edit() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;

    ctx.call("set_memo", json!({ "key": "note", "content": "v1 text" }))
        .await
        .unwrap();
    ctx.call(
        "edit_memo",
        json!({ "key": "note", "old": "v1", "new": "v2" }),
    )
    .await
    .unwrap();

    let content = ctx
        .call("get_memo", json!({ "key": "note" }))
        .await
        .unwrap();
    assert_eq!(content, "v2 text");

    assert_eq!(
        memo_history(dir.path(), "note"),
        vec![Some(b"v2 text".to_vec()), Some(b"v1 text".to_vec()), None]
    );
    assert!(!dir.path().join("backup").exists());
}

#[tokio::test]
async fn edit_memo_should_fail_for_empty_old() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;

    ctx.call("set_memo", json!({ "key": "doc", "content": "abc" }))
        .await
        .unwrap();
    let err = ctx
        .call("edit_memo", json!({ "key": "doc", "old": "", "new": "x" }))
        .await
        .unwrap_err();
    assert!(err.contains("old must not be empty"));
}

#[tokio::test]
async fn edit_memo_should_preserve_validation_order() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;
    std::fs::create_dir(dir.path().join("unreadable.txt")).unwrap();

    for key in ["", "invalid/key", "missing", "unreadable"] {
        let err = ctx
            .call("edit_memo", json!({ "key": key, "old": "", "new": "x" }))
            .await
            .unwrap_err();
        assert_eq!(err, "old must not be empty");
    }

    for (key, expected) in [
        ("", "key must not be empty"),
        (
            "invalid/key",
            "key must contain only alphanumeric characters, hyphens, underscores, or dots",
        ),
        ("missing", "memo 'missing' not found"),
        ("unreadable", "failed to read memo 'unreadable'"),
    ] {
        let err = ctx
            .call("edit_memo", json!({ "key": key, "old": "x", "new": "y" }))
            .await
            .unwrap_err();
        assert_eq!(err, expected);
    }
    assert!(!dir.path().join("missing.txt").exists());
    assert!(dir.path().join("unreadable.txt").is_dir());
    assert!(!dir.path().join("backup").exists());
}

#[tokio::test]
async fn edit_memo_should_leave_memo_and_backups_unchanged_on_failure() {
    for with_backups in [false, true] {
        let dir = tempdir().unwrap();
        let ctx = McpTestContext::new(dir.path().to_path_buf()).await;
        if with_backups {
            for i in 0..5 {
                ctx.call(
                    "set_memo",
                    json!({ "key": "doc", "content": format!("version {i}") }),
                )
                .await
                .unwrap();
            }
        }
        let content = "ああああ\nab ab\naaaa\n末尾";
        ctx.call("set_memo", json!({ "key": "doc", "content": content }))
            .await
            .unwrap();

        let snapshot = || memo_history(dir.path(), "doc");
        let before = snapshot();
        assert_eq!(before.len(), if with_backups { 7 } else { 2 });

        for (old, expected) in [
            ("", "old must not be empty"),
            ("missing", "old text not found in memo 'doc'"),
            ("ab", "old text occurs multiple times in memo 'doc'"),
            ("aaa", "old text occurs multiple times in memo 'doc'"),
            ("あああ", "old text occurs multiple times in memo 'doc'"),
        ] {
            let err = ctx
                .call(
                    "edit_memo",
                    json!({ "key": "doc", "old": old, "new": "replacement" }),
                )
                .await
                .unwrap_err();
            assert_eq!(err, expected);
            assert_eq!(
                std::fs::read(dir.path().join("doc.txt")).unwrap(),
                content.as_bytes()
            );
            assert_eq!(snapshot(), before);
            assert!(!dir.path().join("backup").exists());
        }
    }
}

#[tokio::test]
async fn edit_memo_should_replace_multiline_text() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;

    ctx.call(
        "set_memo",
        json!({ "key": "doc", "content": "header\nbody\nfooter" }),
    )
    .await
    .unwrap();
    let result = ctx
        .call(
            "edit_memo",
            json!({ "key": "doc", "old": "body\nfooter", "new": "newbody\nnewfooter" }),
        )
        .await
        .unwrap();
    assert_eq!(result, "Edited memo 'doc'");

    let content = ctx.call("get_memo", json!({ "key": "doc" })).await.unwrap();
    assert_eq!(content, "header\nnewbody\nnewfooter");
}

#[tokio::test]
async fn edit_memo_should_fail_when_multiline_old_occurs_multiple_times() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;

    ctx.call("set_memo", json!({ "key": "doc", "content": "a\nb\na\nb" }))
        .await
        .unwrap();
    let err = ctx
        .call(
            "edit_memo",
            json!({ "key": "doc", "old": "a\nb", "new": "X" }),
        )
        .await
        .unwrap_err();
    assert!(err.contains("multiple times"));
}

#[tokio::test]
async fn edit_memo_should_fail_when_overlapping_occurrence() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;

    ctx.call("set_memo", json!({ "key": "doc", "content": "ああああ" }))
        .await
        .unwrap();
    let err = ctx
        .call(
            "edit_memo",
            json!({ "key": "doc", "old": "あああ", "new": "X" }),
        )
        .await
        .unwrap_err();
    assert!(err.contains("multiple times"));
}

#[tokio::test]
async fn edit_memo_should_succeed_for_non_overlapping_single() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;

    ctx.call("set_memo", json!({ "key": "doc", "content": "ああああ" }))
        .await
        .unwrap();
    let result = ctx
        .call(
            "edit_memo",
            json!({ "key": "doc", "old": "ああああ", "new": "X" }),
        )
        .await
        .unwrap();
    assert_eq!(result, "Edited memo 'doc'");

    let content = ctx.call("get_memo", json!({ "key": "doc" })).await.unwrap();
    assert_eq!(content, "X");
}
