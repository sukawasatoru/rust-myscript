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

use mcp_memo::MemoServer;
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::{ClientHandler, RoleClient, ServiceExt as _};
use serde_json::json;
use std::path::PathBuf;
use tempfile::tempdir;

#[path = "mcp/delete_memo.rs"]
mod delete_memo;
#[path = "mcp/edit_memo.rs"]
mod edit_memo;
#[path = "mcp/get_memo.rs"]
mod get_memo;
#[path = "mcp/history.rs"]
mod history;
#[path = "mcp/list_memos.rs"]
mod list_memos;
#[path = "mcp/set_memo.rs"]
mod set_memo;

#[derive(Debug, Clone, Default)]
struct DummyClientHandler;
impl ClientHandler for DummyClientHandler {}

struct McpTestContext {
    client: RunningService<RoleClient, DummyClientHandler>,
    server_handle: tokio::task::JoinHandle<()>,
}

impl McpTestContext {
    async fn new(data_dir: PathBuf) -> Self {
        let (server_transport, client_transport) = tokio::io::duplex(4096);
        let server_handle = tokio::spawn(async move {
            MemoServer::new(data_dir)
                .await
                .unwrap()
                .serve(server_transport)
                .await
                .unwrap()
                .waiting()
                .await
                .unwrap();
        });
        let client = DummyClientHandler.serve(client_transport).await.unwrap();
        Self {
            client,
            server_handle,
        }
    }

    async fn call(&self, tool: &str, args: serde_json::Value) -> Result<String, String> {
        let result = self
            .client
            .call_tool(
                CallToolRequestParams::new(tool.to_string())
                    .with_arguments(args.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        let text = result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.to_string())
            .unwrap_or_default();
        if result.is_error.unwrap_or(false) {
            return Err(text);
        }
        Ok(text)
    }
}

fn memo_history(data_dir: &std::path::Path, key: &str) -> Vec<Option<Vec<u8>>> {
    let repo = git2::Repository::open(data_dir).unwrap();
    let mut commit = repo.head().unwrap().peel_to_commit().unwrap();
    let mut history = Vec::new();
    loop {
        let tree = commit.tree().unwrap();
        history.push(
            tree.get_name(&format!("{key}.txt"))
                .map(|entry| repo.find_blob(entry.id()).unwrap().content().to_vec()),
        );
        if commit.parent_count() == 0 {
            break;
        }
        commit = commit.parent(0).unwrap();
    }
    history
}

impl Drop for McpTestContext {
    fn drop(&mut self) {
        self.server_handle.abort();
        self.client.cancellation_token().cancel();
    }
}
