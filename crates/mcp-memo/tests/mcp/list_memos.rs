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
async fn list_memos_should_return_all_keys() {
    let dir = tempdir().unwrap();
    let ctx = McpTestContext::new(dir.path().to_path_buf()).await;

    let result = ctx.call("list_memos", json!({})).await.unwrap();
    assert_eq!(result, "No memos stored.");

    ctx.call("set_memo", json!({ "key": "alpha", "content": "a" }))
        .await
        .unwrap();
    ctx.call("set_memo", json!({ "key": "beta", "content": "b" }))
        .await
        .unwrap();

    let result = ctx.call("list_memos", json!({})).await.unwrap();
    assert_eq!(result, "alpha\nbeta");
}
