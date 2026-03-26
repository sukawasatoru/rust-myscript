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

use clap::{Parser, ValueHint};
use dat_explorer::feature::{read_posts, search_posts};
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use rmcp::schemars::{self, JsonSchema};
use rmcp::{Json, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use rust_myscript::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;
use tracing::Level;

/// 5ch .dat file analysis MCP server
#[derive(Parser)]
struct Opt {
    /// Directory containing dat files
    #[arg(value_hint = ValueHint::DirPath)]
    dat_dir: PathBuf,

    /// max_body_chars の上限キャップ (50000) を無効化する。
    /// LM Studio 以外のクライアントで使用する場合に指定する。
    #[arg(long)]
    disable_body_limit: bool,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_writer(std::io::stderr)
        .init();

    let opt = Opt::parse();
    run_mcp_server(opt.dat_dir, opt.disable_body_limit).await;
}

#[derive(Deserialize, JsonSchema)]
struct ReadPostsToolParams {
    /// ファイル指定（スレ番号 "630" またはファイル名）
    file: String,
    /// レス番号の範囲（例: "1-100", "900-", "-50"）。res_nums と排他
    #[serde(default)]
    range: Option<String>,
    /// 特定のレス番号をリストで指定（例: [86, 87, 99]）。range より優先
    #[serde(default)]
    res_nums: Vec<usize>,
    /// 各レス本文の最大文字数。超過分は切り詰める。0 = 制限なし（デフォルト）
    #[serde(default)]
    max_body_chars: usize,
    /// true の場合 name カラムを含める（デフォルト: false）
    #[serde(default)]
    include_name: bool,
}

#[derive(Deserialize, JsonSchema)]
struct SearchPostsToolParams {
    /// 検索キーワード（正規表現対応）
    #[serde(default)]
    keywords: Vec<String>,
    /// 対象ファイル（スレ番号）。空の場合は全ファイル
    #[serde(default)]
    files: Vec<String>,
    /// レス番号の範囲
    #[serde(default)]
    range: Option<String>,
    /// 投稿者 ID でフィルタ（部分一致）。keywords なしでも使用可能
    #[serde(default)]
    ids: Vec<String>,
    /// ヒット本文の合計文字数の目安上限。超えたレスまで含めて打ち切る。0 = 制限なし（デフォルト）
    #[serde(default)]
    max_body_chars: usize,
}

#[derive(Serialize, JsonSchema)]
struct ReadPostsResponse {
    file_info: FileInfoEntry,
    /// カラム名の一覧: ["res_num", "name", "datetime", "id", "body", "title", "ref_count"]
    columns: Vec<String>,
    /// 各レスの値を columns の順に並べた配列
    rows: Vec<Vec<serde_json::Value>>,
    /// max_body_chars 超過により省略されたレス数
    #[serde(default, skip_serializing_if = "is_zero")]
    omitted_count: usize,
}

fn is_zero(v: &usize) -> bool {
    *v == 0
}

#[derive(Serialize, JsonSchema)]
struct FileInfoEntry {
    filename: String,
    thread_num: u32,
    thread_title: String,
    total_lines: usize,
    date_range: String,
}

#[derive(Serialize, JsonSchema)]
struct SearchPostsResponse {
    total_hits: usize,
    searched_files: Vec<String>,
    /// カラム名の一覧: ["file", "res_num", "datetime", "id", "body", "urls", "matched_keywords", "ref_count"]
    columns: Vec<String>,
    /// 各ヒットの値を columns の順に並べた配列
    rows: Vec<Vec<serde_json::Value>>,
    /// max_body_chars 超過により省略されたヒット数
    #[serde(default, skip_serializing_if = "is_zero")]
    omitted_count: usize,
}

struct McpServer {
    tool_router: ToolRouter<Self>,
    dat_dir: PathBuf,
    disable_body_limit: bool,
}

#[tool_router]
impl McpServer {
    fn new(dat_dir: PathBuf, disable_body_limit: bool) -> Self {
        Self {
            tool_router: Self::tool_router(),
            dat_dir,
            disable_body_limit,
        }
    }

    /// 指定ファイルのレスを読み取る。範囲指定や特定レス番号指定が可能
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn read_posts(
        &self,
        params: Parameters<ReadPostsToolParams>,
    ) -> Result<Json<ReadPostsResponse>, String> {
        let p = &params.0;
        let result = read_posts::read_posts(
            &self.dat_dir,
            &read_posts::ReadPostsParams {
                file: p.file.clone(),
                range: p.range.clone(),
                res_nums: p.res_nums.clone(),
                max_body_chars: p.max_body_chars,
                include_name: p.include_name,
                disable_body_limit: self.disable_body_limit,
            },
        )
        .map_err(|e| e.to_string())?;

        let ref_counts = result.ref_counts;
        let include_name = p.include_name;
        let mut columns = vec!["res_num".into(), "datetime".into(), "id".into()];
        if include_name {
            columns.insert(1, "name".into());
        }
        columns.extend(["body".into(), "title".into(), "ref_count".into()]);
        let rows = result
            .posts
            .into_iter()
            .map(|post| {
                let ref_count = ref_counts.get(&post.res_num).copied().unwrap_or(0);
                let mut row = vec![json!(post.res_num)];
                if include_name {
                    row.push(json!(post.name));
                }
                row.extend([
                    json!(post.datetime),
                    json!(post.id),
                    json!(post.body),
                    json!(post.title),
                    json!(ref_count),
                ]);
                row
            })
            .collect();
        Ok(Json(ReadPostsResponse {
            file_info: FileInfoEntry {
                filename: result.file_info.filename,
                thread_num: result.file_info.thread_num,
                thread_title: result.file_info.thread_title,
                total_lines: result.file_info.total_lines,
                date_range: result.file_info.date_range,
            },
            columns,
            rows,
            omitted_count: result.omitted_count,
        }))
    }

    /// キーワード（正規表現）または投稿者 ID でレスを検索する
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn search_posts(
        &self,
        params: Parameters<SearchPostsToolParams>,
    ) -> Result<Json<SearchPostsResponse>, String> {
        let p = &params.0;

        if p.keywords.is_empty() && p.ids.is_empty() {
            return Err("keywords, ids のいずれかを指定してください".into());
        }

        let result = search_posts::search_posts(
            &self.dat_dir,
            &search_posts::SearchPostsParams {
                keywords: p.keywords.clone(),
                files: p.files.clone(),
                range: p.range.clone(),
                ids: p.ids.clone(),
                max_body_chars: p.max_body_chars,
                disable_body_limit: self.disable_body_limit,
            },
        )
        .map_err(|e| e.to_string())?;

        let columns = vec![
            "file".into(),
            "res_num".into(),
            "datetime".into(),
            "id".into(),
            "body".into(),
            "urls".into(),
            "matched_keywords".into(),
            "ref_count".into(),
        ];
        let rows = result
            .hits
            .into_iter()
            .map(|h| {
                vec![
                    json!(h.file),
                    json!(h.res_num),
                    json!(h.datetime),
                    json!(h.id),
                    json!(h.body),
                    json!(h.urls),
                    json!(h.matched_keywords),
                    json!(h.ref_count),
                ]
            })
            .collect();
        Ok(Json(SearchPostsResponse {
            total_hits: result.total_hits,
            searched_files: result.searched_files,
            columns,
            rows,
            omitted_count: result.omitted_count,
        }))
    }
}

#[tool_handler]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")),
        )
    }
}

async fn run_mcp_server(dat_dir: PathBuf, disable_body_limit: bool) {
    let server = McpServer::new(dat_dir, disable_body_limit).serve(rmcp::transport::stdio());
    let running = match server.await {
        Ok(running) => running,
        Err(e) => {
            error!(?e, "failed to initialize MCP server");
            return;
        }
    };

    if let Err(e) = running.waiting().await {
        error!(?e, "MCP server task panicked");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use rmcp::model::CallToolRequestParams;
    use rmcp::service::RunningService;
    use rmcp::{ClientHandler, RoleClient, ServiceExt};
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn struct_opt() {
        Opt::command().debug_assert();
    }

    // NOTE: dat::test_helpers is behind #[cfg(test)] + tempfile (dev-dep),
    // so it cannot be referenced from the binary crate. Test data is defined independently.
    struct TestDirs {
        _dir: TempDir,
        dat_dir: PathBuf,
    }

    fn create_test_dirs() -> TestDirs {
        let dir = TempDir::new().unwrap();
        let dat_dir = dir.path().join("dat_files");
        std::fs::create_dir_all(&dat_dir).unwrap();

        let dat_630 = [
            "テスト名<>sage<>2026/03/13(金) 10:38:56.82 ID:test0001<>最初のレス https://example.com/image001.jpg ここまで<>テストスレッド★630",
            "名無し<><>2026/03/13(金) 11:00:00.00 ID:test0002<>Tool v2.5すごい<br>&gt;&gt;1 これは便利<>",
            "名無し<>sage<>2026/03/13(金) 12:00:00.00 ID:test0003<>https://example.com/resources/12345 新しいプラグインが公開された<>",
            "名無し<><>2026/03/14(土) 09:00:00.00 ID:test0004<>App-X試してみた https://example.com/files/demo.mp4<>",
            "名無し<>sage<>2026/03/14(土) 10:00:00.00 ID:test0005<>https://example.com/repo/test Widget-Yも気になる<>",
        ].join("\n");
        std::fs::write(dat_dir.join("board_630_1773365936.dat"), &dat_630).unwrap();

        let dat_631 = [
            "テスト<>sage<>2026/03/18(水) 20:03:27.00 ID:test0010<>新スレ立てた<>テストスレッド★631",
            "名無し<><>2026/03/18(水) 21:00:00.00 ID:test0011<>Foobarで生成してみた https://example.com/output/xyz.png<>",
            "名無し<><>2026/03/18(水) 22:00:00.00 ID:test0012<>Bazqux<>",
        ].join("\n");
        std::fs::write(dat_dir.join("board_631_1773831807.dat"), &dat_631).unwrap();

        TestDirs { _dir: dir, dat_dir }
    }

    #[derive(Debug, Clone, Default)]
    struct DummyClientHandler;
    impl ClientHandler for DummyClientHandler {}

    struct McpTestContext {
        client: RunningService<RoleClient, DummyClientHandler>,
        server_handle: tokio::task::JoinHandle<anyhow::Result<()>>,
    }

    impl McpTestContext {
        async fn new(dat_dir: PathBuf) -> Fallible<Self> {
            let (server_transport, client_transport) = tokio::io::duplex(4096);
            let server_handle = tokio::spawn(async move {
                McpServer::new(dat_dir, false)
                    .serve(server_transport)
                    .await?
                    .waiting()
                    .await?;
                anyhow::Ok(())
            });
            let client = DummyClientHandler.serve(client_transport).await?;
            Ok(Self {
                client,
                server_handle,
            })
        }

        async fn call(&self, tool: &str, args: serde_json::Value) -> Fallible<serde_json::Value> {
            let tool_name = tool.to_string();
            let result = self
                .client
                .call_tool(
                    CallToolRequestParams::new(tool_name)
                        .with_arguments(args.as_object().unwrap().clone()),
                )
                .await?;

            if result.is_error.unwrap_or(false) {
                let text = result
                    .content
                    .first()
                    .and_then(|c| c.raw.as_text())
                    .map(|t| t.text.to_string())
                    .unwrap_or_default();
                bail!("tool error: {text}");
            }

            // Prefer structured_content if available
            if let Some(structured) = result.structured_content {
                return Ok(structured);
            }

            // Fallback: parse text from content
            let text = result
                .content
                .first()
                .and_then(|c| c.raw.as_text())
                .map(|t| t.text.to_string())
                .unwrap_or_default();
            Ok(serde_json::from_str(&text)?)
        }
    }

    impl Drop for McpTestContext {
        fn drop(&mut self) {
            self.server_handle.abort();
            self.client.cancellation_token().cancel();
        }
    }

    #[tokio::test]
    async fn mcp_read_posts() -> Fallible<()> {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone()).await?;

        let parsed = ctx
            .call("read_posts", json!({ "file": "630", "range": "1-2" }))
            .await?;
        assert_eq!(parsed["rows"].as_array().unwrap().len(), 2);
        assert!(parsed["columns"].as_array().unwrap().len() > 0);
        assert_eq!(parsed["file_info"]["thread_num"], 630);
        assert!(parsed["file_info"]["date_range"].is_string());
        Ok(())
    }

    #[tokio::test]
    async fn mcp_search_posts_with_keywords() -> Fallible<()> {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone()).await?;

        let parsed = ctx
            .call("search_posts", json!({ "keywords": ["Tool v2\\.5"] }))
            .await?;
        assert_eq!(parsed["total_hits"], 1);
        Ok(())
    }

    #[tokio::test]
    async fn mcp_search_posts_no_keywords_error() -> Fallible<()> {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone()).await?;

        let result = ctx.call("search_posts", json!({})).await;
        assert!(result.is_err());
        Ok(())
    }
}
