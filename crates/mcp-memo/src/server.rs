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

use crate::feature;
use crate::store::MemoStore;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use rmcp::schemars::{self, JsonSchema};
use rmcp::{ServerHandler, ServiceExt as _, tool, tool_handler, tool_router};
use rust_myscript::prelude::*;
use serde::Deserialize;
use std::path::PathBuf;
use tracing::instrument;

pub async fn run_mcp_server(data_dir: PathBuf) -> Fallible<()> {
    info!(data_dir = %data_dir.display(), "data directory");
    let running = MemoServer::new(data_dir)
        .await?
        .serve(rmcp::transport::stdio())
        .await
        .context("failed to initialize MCP server")?;
    running
        .waiting()
        .await
        .context("MCP server task panicked")?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct MemoServer {
    store: MemoStore,

    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl MemoServer {
    pub async fn new(data_dir: PathBuf) -> Fallible<Self> {
        let store = MemoStore::new(data_dir);
        store.initialize().await?;
        Ok(Self {
            store,
            tool_router: Self::tool_router(),
        })
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

#[derive(Debug, Deserialize, JsonSchema)]
struct EditMemoRequest {
    /// The key of the memo to edit.
    key: String,
    /// The string to find and replace (must occur exactly once).
    old: String,
    /// The replacement string.
    new: String,
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
        feature::get_memo(&self.store, &req.key)
            .await
            .map_err(|e| e.to_string())
    }

    /// Store content into a memo by key.
    #[tool]
    #[instrument(skip(self))]
    async fn set_memo(
        &self,
        Parameters(req): Parameters<SetMemoRequest>,
    ) -> Result<String, String> {
        feature::set_memo(&self.store, &req.key, &req.content)
            .await
            .map_err(|e| e.to_string())?;
        Ok(format!("Stored memo '{}'", req.key))
    }

    /// Delete a memo by key.
    #[tool]
    #[instrument(skip(self))]
    async fn delete_memo(
        &self,
        Parameters(req): Parameters<DeleteMemoRequest>,
    ) -> Result<String, String> {
        feature::delete_memo(&self.store, &req.key)
            .await
            .map_err(|e| e.to_string())?;
        Ok(format!("Deleted memo '{}'", req.key))
    }

    /// Edit a memo by replacing a single occurrence of `old` with `new`.
    #[tool]
    #[instrument(skip(self))]
    async fn edit_memo(
        &self,
        Parameters(req): Parameters<EditMemoRequest>,
    ) -> Result<String, String> {
        feature::edit_memo(&self.store, &req.key, &req.old, &req.new)
            .await
            .map_err(|e| e.to_string())?;
        Ok(format!("Edited memo '{}'", req.key))
    }

    /// List all memo keys.
    #[tool]
    #[instrument(skip(self))]
    async fn list_memos(&self) -> Result<String, String> {
        let keys = feature::list_memos(&self.store)
            .await
            .map_err(|e| e.to_string())?;
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
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
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
