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

//! Long-lived local child process. Only local listing and reading are exposed here.

use crate::dto::mcp::{ListThreadsResponse, ReadPostsResponse};
use crate::model::viewer::{ThreadEntry, ThreadSnapshot, ViewerPost};
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::{Peer, RoleClient, ServiceExt};
use rust_myscript::prelude::*;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::sync::Mutex;

pub struct McpClient {
    peer: Peer<RoleClient>,
    running: Mutex<RunningService<RoleClient, ()>>,
}

impl McpClient {
    pub async fn connect(executable: &Path, dat_dir: &Path) -> Fallible<Self> {
        let mut command = tokio::process::Command::new(executable);
        command.arg("mcp").arg(dat_dir).arg("--disable-body-limit");
        let (transport, _) = rmcp::transport::TokioChildProcess::builder(command)
            .stderr(Stdio::null())
            .spawn()?;
        let running = tokio::time::timeout(Duration::from_secs(10), ().serve(transport))
            .await
            .context("MCP initialization timed out")??;
        Ok(Self {
            peer: running.peer().clone(),
            running: Mutex::new(running),
        })
    }

    pub async fn close(&self) -> Fallible<()> {
        self.running.lock().await.close().await?;
        Ok(())
    }

    async fn call<T: DeserializeOwned>(&self, name: &'static str, arguments: Value) -> Fallible<T> {
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            self.peer.call_tool(
                CallToolRequestParams::new(name).with_arguments(
                    arguments
                        .as_object()
                        .context("tool arguments must be an object")?
                        .clone(),
                ),
            ),
        )
        .await
        .context("MCP request timed out")??;
        if result.is_error == Some(true) {
            bail!("MCP {name}: {:?}", result.content);
        }
        let value = result
            .structured_content
            .context("MCP response has no structured content")?;
        Ok(serde_json::from_value(value)?)
    }

    pub async fn list_threads(&self) -> Fallible<Vec<ThreadEntry>> {
        let response: ListThreadsResponse = self.call("list_threads", json!({})).await?;
        Ok(response
            .threads
            .into_iter()
            .map(|entry| ThreadEntry {
                file: entry.file,
                title: entry.title,
                post_count: entry.post_count,
                created_at: entry.created_at,
            })
            .collect())
    }

    pub async fn load_thread(&self, file: &str) -> Fallible<ThreadSnapshot> {
        let response = self
            .call(
                "read_posts",
                json!({
                    "file": file, "include_name": true, "include_id": true, "max_body_chars": 0,
                }),
            )
            .await?;
        decode_snapshot(response)
    }
}

fn decode_snapshot(response: ReadPostsResponse) -> Fallible<ThreadSnapshot> {
    ensure!(response.omitted_count == 0, "MCP response omitted posts");
    let column = |name| {
        response
            .columns
            .iter()
            .position(|c| c == name)
            .with_context(|| format!("missing MCP column: {name}"))
    };
    let number = column("res_num")?;
    let name = column("name")?;
    let datetime = column("datetime")?;
    let id = column("id")?;
    let body = column("body")?;
    let mut posts = Vec::new();
    for row in response.rows {
        ensure!(
            row.len() == response.columns.len(),
            "invalid MCP row length"
        );
        let text = |index: usize| {
            row[index]
                .as_str()
                .map(str::to_owned)
                .context("invalid MCP text cell")
        };
        let number = usize::try_from(row[number].as_u64().context("invalid MCP post number")?)?;
        ensure!(
            number > 0 && posts.last().is_none_or(|p: &ViewerPost| p.number < number),
            "MCP posts must be ordered by number"
        );
        posts.push(ViewerPost {
            number,
            name: text(name)?,
            datetime: text(datetime)?,
            id: text(id)?,
            body: text(body)?,
        });
    }
    Ok(ThreadSnapshot {
        title: response.file_info.thread_title,
        posts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_incomplete_or_malformed_snapshots() {
        let valid = json!({
            "file_info": { "filename": "board_1_123.dat", "thread_num": 1,
                "thread_title": "title", "total_lines": 1, "date_range": "" },
            "columns": ["body", "id", "datetime", "name", "res_num"],
            "rows": [["本文", "abc", "2026/09/27", "名無し", 3]],
        });
        let decoded = decode_snapshot(serde_json::from_value(valid.clone()).unwrap()).unwrap();
        assert_eq!(decoded.posts[0].number, 3);
        assert_eq!(decoded.posts[0].body, "本文");
        let mut truncated = valid.clone();
        truncated["omitted_count"] = json!(1);
        assert!(decode_snapshot(serde_json::from_value(truncated).unwrap()).is_err());
        let mut missing_column = valid.clone();
        missing_column["columns"][3] = json!("other");
        assert!(decode_snapshot(serde_json::from_value(missing_column).unwrap()).is_err());
        let mut malformed = valid.clone();
        malformed["rows"][0] = json!(["本文"]);
        assert!(decode_snapshot(serde_json::from_value(malformed).unwrap()).is_err());
        let mut unordered = valid.clone();
        unordered["rows"]
            .as_array_mut()
            .unwrap()
            .push(json!(["本文", "abc", "date", "name", 2]));
        assert!(decode_snapshot(serde_json::from_value(unordered).unwrap()).is_err());
    }
}
