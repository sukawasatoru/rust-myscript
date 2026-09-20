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
async fn get_memo_should_return_stored_content() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;

    ctx.call("set_memo", json!({ "key": "hello", "content": "world" }))
        .await
        .unwrap();
    let result = ctx
        .call("get_memo", json!({ "key": "hello" }))
        .await
        .unwrap();
    assert_eq!(result, "world");
}

#[tokio::test]
async fn get_memo_should_fail_for_invalid_key() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;

    ctx.call("get_memo", json!({ "key": "../etc/passwd" }))
        .await
        .unwrap_err();
}
