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

use dat_explorer::application::backend::Backend;
use dat_explorer::application::thread_service::ThreadService;
use dat_explorer::data::mcp_client::McpClient;
use std::path::Path;
use std::sync::Arc;

#[tokio::test]
async fn local_and_mcp_paths_agree_and_reload_without_truncation() {
    let dir = tempfile::TempDir::new().unwrap();
    let body = format!("{}<br>&gt;&gt;3 👨‍👩‍👧‍👦", "長い本文".repeat(20000));
    let text = format!(
        "<b>名前</b><>sage<>2026/09/27 ID:abc<>{body}<>タイトル\ninvalid\n名無し<><>2026/09/28 ID:def<>二番目<>\nOver 1000 Thread\n"
    );
    for file in [
        "board_1_unknown.dat",
        "board_2_100.dat",
        "board_3_200.dat",
        "board_4_200.dat",
    ] {
        std::fs::write(dir.path().join(file), &text).unwrap();
    }
    std::fs::create_dir(dir.path().join("board_5_300.dat")).unwrap();
    let direct = Backend::Direct(Arc::new(ThreadService::new(
        dir.path().to_path_buf(),
        reqwest::Client::new(),
    )));
    let client = Arc::new(
        McpClient::connect(Path::new(env!("CARGO_BIN_EXE_dat-explorer")), dir.path())
            .await
            .unwrap(),
    );
    let mcp = Backend::Mcp(client.clone());
    let entries = direct.clone().list_threads().await.unwrap();
    assert_eq!(
        entries.iter().map(|e| e.file.as_str()).collect::<Vec<_>>(),
        [
            "board_3_200.dat",
            "board_4_200.dat",
            "board_2_100.dat",
            "board_1_unknown.dat"
        ]
    );
    assert_eq!(entries, mcp.clone().list_threads().await.unwrap());
    let file = entries[0].file.clone();
    let snapshot = direct.clone().load_thread(file.clone()).await.unwrap();
    assert_eq!(
        snapshot,
        mcp.clone().load_thread(file.clone()).await.unwrap()
    );
    assert_eq!(snapshot.posts.len(), 2);
    assert_eq!(snapshot.posts[1].number, 3);
    assert_eq!(snapshot.posts[0].name, "名前");
    assert!(snapshot.posts[0].body.len() > 50000);
    std::fs::write(
        dir.path().join(&file),
        "名無し<><>2026/09/29 ID:x<>更新済み<>新しいタイトル\n",
    )
    .unwrap();
    let updated = mcp.clone().load_thread(file.clone()).await.unwrap();
    assert_eq!(updated.posts.len(), 1);
    assert_eq!(updated.posts[0].body, "更新済み");
    assert_eq!(
        updated,
        direct.clone().load_thread(file.clone()).await.unwrap()
    );
    assert_eq!(
        direct.clone().list_threads().await.unwrap(),
        mcp.clone().list_threads().await.unwrap()
    );
    assert!(direct.load_thread("missing.dat".into()).await.is_err());
    assert!(mcp.load_thread("missing.dat".into()).await.is_err());
    client.close().await.unwrap();
    assert!(client.list_threads().await.is_err());
}
