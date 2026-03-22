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

use chrono::Local;
use clap::{Parser, ValueHint};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerInfo};
use rmcp::schemars::{self, JsonSchema};
use rmcp::{ServerHandler, ServiceExt as _, tool, tool_handler, tool_router};
use rust_myscript::prelude::*;
use serde::Deserialize;
use std::path::PathBuf;
use tracing::{Level, instrument};

const MAX_BACKUP_COUNT: usize = 5;

#[derive(Debug, Parser)]
struct Opt {
    /// Directory to store memos.
    #[clap(value_hint = ValueHint::DirPath)]
    data_dir: PathBuf,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(Level::INFO)
        .init();

    let opt = Opt::parse();
    let data_dir = opt.data_dir;
    if let Err(e) = tokio::fs::create_dir_all(&data_dir).await {
        error!(?e, path = %data_dir.display(), "failed to create data directory");
        return;
    }

    info!(data_dir = %data_dir.display(), "data directory");

    let running = match MemoServer::new(data_dir)
        .serve(rmcp::transport::stdio())
        .await
    {
        Ok(r) => r,
        Err(e) => {
            error!(?e, "failed to initialize MCP server");
            return;
        }
    };
    if let Err(e) = running.waiting().await {
        error!(?e, "MCP server task panicked");
    }
}

#[derive(Debug, Clone)]
struct MemoServer {
    data_dir: PathBuf,
    tool_router: ToolRouter<Self>,
}

impl MemoServer {
    fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            tool_router: Self::tool_router(),
        }
    }

    fn backup_dir(&self, key: &str) -> PathBuf {
        self.data_dir.join("backup").join(key)
    }

    async fn backup_memo(&self, path: &std::path::Path, key: &str) {
        let backup_dir = self.backup_dir(key);
        if let Err(e) = tokio::fs::create_dir_all(&backup_dir).await {
            warn!(?e, %key, "failed to create backup directory");
            return;
        }
        // Nanosecond precision makes collisions extremely unlikely in practice.
        // If collisions become a concern, consider using UUIDs or a sequence number instead.
        let timestamp = Local::now().format("%Y%m%d_%H%M%S_%9f").to_string();
        let backup_path = backup_dir.join(format!("{timestamp}.txt"));
        if let Err(e) = tokio::fs::rename(path, &backup_path).await {
            warn!(?e, %key, "failed to backup memo");
            return;
        }
        // Prune old backups beyond MAX_BACKUP_COUNT
        let Ok(mut entries) = tokio::fs::read_dir(&backup_dir).await else {
            return;
        };
        let mut names = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            names.push(entry.file_name().to_string_lossy().to_string());
        }
        names.sort();
        for name in names
            .iter()
            .take(names.len().saturating_sub(MAX_BACKUP_COUNT))
        {
            let old = backup_dir.join(name);
            if let Err(e) = tokio::fs::remove_file(&old).await {
                warn!(?e, %key, "failed to remove old backup");
            }
        }
    }

    fn key_to_path(&self, key: &str) -> Fallible<PathBuf> {
        // Validate key: only alphanumeric, hyphens, underscores, and dots allowed
        if key.is_empty() {
            bail!("key must not be empty");
        }
        if !key
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            bail!("key must contain only alphanumeric characters, hyphens, underscores, or dots");
        }
        Ok(self.data_dir.join(format!("{key}.txt")))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetMemoRequest {
    /// The key of the memo to retrieve.
    key: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SetMemoRequest {
    /// The key of the memo to store.
    key: String,
    /// The content to store.
    content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DeleteMemoRequest {
    /// The key of the memo to delete.
    key: String,
}

#[tool_router]
impl MemoServer {
    /// Get the content of a memo by key.
    #[tool]
    #[instrument(skip(self))]
    async fn get_memo(
        &self,
        Parameters(req): Parameters<GetMemoRequest>,
    ) -> Result<String, String> {
        let path = self.key_to_path(&req.key).map_err(|e| e.to_string())?;
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => Ok(content),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(format!("memo '{}' not found", req.key))
            }
            Err(e) => {
                warn!(?e, key = %req.key, "failed to read memo");
                Err(format!("failed to read memo '{}'", req.key))
            }
        }
    }

    /// Store content into a memo by key.
    #[tool]
    #[instrument(skip(self))]
    async fn set_memo(
        &self,
        Parameters(req): Parameters<SetMemoRequest>,
    ) -> Result<String, String> {
        let path = self.key_to_path(&req.key).map_err(|e| e.to_string())?;
        if tokio::fs::metadata(&path).await.is_ok() {
            // TOCTOU: the file could be removed between the metadata check and rename inside
            // backup_memo, but rename's NotFound is already handled gracefully there, so this
            // is acceptable for now.
            // backup_memo failures are intentionally non-fatal: a warn! is emitted and the
            // write proceeds so that memo updates are never blocked by backup errors.
            self.backup_memo(&path, &req.key).await;
        }
        if let Err(e) = tokio::fs::write(&path, &req.content).await {
            warn!(?e, key = %req.key, "failed to write memo");
            return Err(format!("failed to write memo '{}'", req.key));
        }
        Ok(format!("Stored memo '{}'", req.key))
    }

    /// Delete a memo by key.
    #[tool]
    #[instrument(skip(self))]
    async fn delete_memo(
        &self,
        Parameters(req): Parameters<DeleteMemoRequest>,
    ) -> Result<String, String> {
        let path = self.key_to_path(&req.key).map_err(|e| e.to_string())?;
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(format!("Deleted memo '{}'", req.key)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(format!("memo '{}' not found", req.key))
            }
            Err(e) => {
                warn!(?e, key = %req.key, "failed to delete memo");
                Err(format!("failed to delete memo '{}'", req.key))
            }
        }
    }

    /// List all memo keys.
    #[tool]
    #[instrument(skip(self))]
    async fn list_memos(&self) -> Result<String, String> {
        let mut entries = match tokio::fs::read_dir(&self.data_dir).await {
            Ok(e) => e,
            Err(e) => {
                warn!(?e, "failed to read data directory");
                return Err("failed to read memo list".to_string());
            }
        };
        let mut keys = Vec::new();
        loop {
            match entries.next_entry().await {
                Ok(Some(entry)) => {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    // Check file type to avoid accidentally listing subdirectories (e.g. "backup")
                    // that might have a .txt suffix in the future.
                    let is_file = entry
                        .file_type()
                        .await
                        .map(|t| t.is_file())
                        .unwrap_or(false);
                    if is_file && let Some(key) = name.strip_suffix(".txt") {
                        keys.push(key.to_string());
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    warn!(?e, "failed to read directory entry");
                    return Err("failed to read memo list".to_string());
                }
            }
        }
        keys.sort();
        if keys.is_empty() {
            Ok("No memos stored.".to_string())
        } else {
            Ok(keys.join("\n"))
        }
    }
}

#[tool_handler]
impl ServerHandler for MemoServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::default()
            .with_server_info(Implementation::new(
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "A memo server for storing and retrieving temporary notes by key. \
                Useful for preserving context, intermediate results, or reminders across tasks.",
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use rmcp::model::CallToolRequestParams;
    use rmcp::service::RunningService;
    use rmcp::{ClientHandler, RoleClient};
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn opt_should_parse_valid_args() {
        Opt::command().debug_assert();
    }

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
                .and_then(|c| c.raw.as_text())
                .map(|t| t.text.to_string())
                .unwrap_or_default();
            if result.is_error.unwrap_or(false) {
                return Err(text);
            }
            Ok(text)
        }
    }

    impl Drop for McpTestContext {
        fn drop(&mut self) {
            self.server_handle.abort();
            self.client.cancellation_token().cancel();
        }
    }

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

        // Backup directory should contain one file with content v1
        let backup_dir = dir.path().join("backup").join("note");
        let mut entries = std::fs::read_dir(&backup_dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        assert_eq!(entries.len(), 1);
        assert_eq!(std::fs::read_to_string(&entries[0]).unwrap(), "v1");
    }

    #[tokio::test]
    async fn set_memo_should_prune_old_backups_beyond_max_count() {
        let dir = tempdir().unwrap();
        let ctx = McpTestContext::new(dir.path().to_path_buf()).await;

        // Write 7 times; backup should be capped at MAX_BACKUP_COUNT (5)
        for i in 1..=7u32 {
            ctx.call(
                "set_memo",
                json!({ "key": "prune", "content": format!("v{i}") }),
            )
            .await
            .unwrap();
        }

        let backup_dir = dir.path().join("backup").join("prune");
        let count = std::fs::read_dir(&backup_dir).unwrap().count();
        assert_eq!(count, 5);
    }

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
}
