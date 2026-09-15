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
use dat_explorer::feature::{fetch_dat, fetch_subject, read_posts, search_posts};
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use rmcp::schemars::{self, JsonSchema};
use rmcp::{Json, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use rust_myscript::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};
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
    /// true の場合 id カラムを含める（デフォルト: false）
    #[serde(default)]
    include_id: bool,
    /// true の場合 urls カラムを含める（デフォルト: false）
    #[serde(default)]
    include_urls: bool,
}

#[derive(Serialize, JsonSchema)]
struct ReadPostsResponse {
    file_info: FileInfoEntry,
    /// カラム名の一覧: ["res_num", "name", "datetime", "id", "body", "ref_count", "urls"] (name, id, urls は引数による)
    columns: Vec<String>,
    /// 各レスの値を columns の順に並べた配列
    rows: Vec<Vec<serde_json::Value>>,
    /// max_body_chars 超過により省略されたレス数
    #[serde(default, skip_serializing_if = "is_zero")]
    omitted_count: usize,
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
    /// true の場合 id カラムを含める（デフォルト: false）
    #[serde(default)]
    include_id: bool,
}

#[derive(Serialize, JsonSchema)]
struct SearchPostsResponse {
    total_hits: usize,
    searched_files: Vec<String>,
    /// カラム名の一覧: ["file", "res_num", "datetime", "id", "body", "urls", "ref_count"] (id は引数による)
    columns: Vec<String>,
    /// 各ヒットの値を columns の順に並べた配列
    rows: Vec<Vec<serde_json::Value>>,
    /// max_body_chars 超過により省略されたヒット数
    #[serde(default, skip_serializing_if = "is_zero")]
    omitted_count: usize,
}

#[derive(Deserialize, JsonSchema)]
struct FetchDatToolParams {
    /// スレッドの URL。以下の2形式に対応する。どちらの形式でも、まず dat 直接取得を試み、
    /// 404（dat落ち）の場合は自動的に read.cgi 経由（HTML取得・dat変換）に切り替える。
    ///
    /// 形式1 - dat URL（現行スレ・dat落ちスレ共通）:
    ///   "https://{server}.5ch.io/{board}/dat/{thread_id}.dat"
    ///
    /// 形式2 - read.cgi URL:
    ///   "https://{server}.5ch.io/test/read.cgi/{board}/{thread_id}/"
    url: String,

    /// 保存先ファイルパス。絶対パスまたは相対パスで指定する。
    /// 相対パスの場合は CLI 引数で指定した dat_dir を基準に解決する。
    /// 既存ファイルがある場合は上書き保存し、増加レス数を added_res_count で返す。
    /// 例（絶対パス）: "/path/to/dir/PREFIX_635_1234567890.dat"
    /// 例（相対パス）: "PREFIX_635_1234567890.dat"
    save_path: String,
}

#[derive(Serialize, JsonSchema)]
struct FetchDatResponse {
    /// 実際に保存したファイルの絶対パス
    save_path: String,
    /// 保存した dat のレス数（空行を除く行数）
    res_count: usize,
    /// 既存ファイルからの増加レス数。既存ファイルがなければ null、更新がなければ 0
    added_res_count: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct FetchSubjectToolParams {
    /// subject.txt の URL。http または https のみ指定できる。
    /// userinfo は指定可能。fragment は除去し、query は保持する。
    /// 例: "https://fate.5ch.io/liveuranus/subject.txt"
    url: String,

    /// 指定した場合、スレッドタイトルに含まれるものだけを返す。
    /// NCR 復元後のタイトルへの大小文字無視の部分一致。未指定または空文字は全件を返す。
    #[serde(default)]
    title_contains: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct FetchSubjectResponse {
    /// subject.txt に現れる順のスレッド一覧
    threads: Vec<SubjectThreadEntry>,
}

#[derive(Serialize, JsonSchema)]
struct SubjectThreadEntry {
    /// スレッドキー。先頭ゼロを保持するため文字列で返す
    thread_id: String,
    /// スレッドタイトル（NCR 復元後、trim しない）
    title: String,
    /// レス数
    res_count: u64,
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

struct McpServer {
    tool_router: ToolRouter<Self>,
    dat_dir: PathBuf,
    disable_body_limit: bool,
    http_client: reqwest::Client,
}

#[tool_router]
impl McpServer {
    fn new(dat_dir: PathBuf, disable_body_limit: bool, http_client: reqwest::Client) -> Self {
        Self {
            tool_router: Self::tool_router(),
            dat_dir,
            disable_body_limit,
            http_client,
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
                include_id: p.include_id,
                include_urls: p.include_urls,
                disable_body_limit: self.disable_body_limit,
            },
        )
        .map_err(|e| {
            warn!(?e, "read_posts failed");
            e.to_string()
        })?;

        let ref_counts = result.ref_counts;
        let urls = result.urls;
        let include_name = p.include_name;
        let include_id = p.include_id;
        let include_urls = p.include_urls;
        let mut columns = vec!["res_num".into(), "datetime".into()];
        if include_name {
            columns.insert(1, "name".into());
        }
        if include_id {
            columns.push("id".into());
        }
        columns.extend(["body".into(), "ref_count".into()]);
        if include_urls {
            columns.push("urls".into());
        }
        let rows = result
            .posts
            .into_iter()
            .map(|post| {
                let ref_count = ref_counts.get(&post.res_num).copied().unwrap_or(0);
                let post_urls = urls.get(&post.res_num);
                let mut row = vec![json!(post.res_num)];
                if include_name {
                    row.push(json!(post.name));
                }
                row.push(json!(post.datetime));
                if include_id {
                    row.push(json!(post.id));
                }
                row.extend([json!(post.body), json!(ref_count)]);
                if include_urls {
                    row.push(json!(post_urls));
                }
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
                include_id: p.include_id,
                disable_body_limit: self.disable_body_limit,
            },
        )
        .map_err(|e| {
            warn!(?e, "search_posts failed");
            e.to_string()
        })?;

        let include_id = p.include_id;
        let mut columns = vec!["file".into(), "res_num".into(), "datetime".into()];
        if include_id {
            columns.push("id".into());
        }
        columns.extend(["body".into(), "urls".into(), "ref_count".into()]);
        let rows = result
            .hits
            .into_iter()
            .map(|h| {
                let mut row = vec![json!(h.file), json!(h.res_num), json!(h.datetime)];
                if include_id {
                    row.push(json!(h.id));
                }
                row.extend([json!(h.body), json!(h.urls), json!(h.ref_count)]);
                row
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

    /// 5ch のスレッドをインターネットから取得して UTF-8 の dat ファイルとして保存する
    #[tool(annotations(read_only_hint = false, open_world_hint = true))]
    async fn fetch_dat(
        &self,
        params: Parameters<FetchDatToolParams>,
    ) -> Result<Json<FetchDatResponse>, String> {
        let p = &params.0;
        let save_path = {
            let p = Path::new(&p.save_path);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                self.dat_dir.join(p)
            }
        };
        let result = fetch_dat::fetch_dat(&fetch_dat::FetchDatParams {
            url: p.url.clone(),
            save_path: save_path.to_string_lossy().into_owned(),
        })
        .await
        .map_err(|e| {
            warn!(?e, "fetch_dat failed");
            e.to_string()
        })?;
        Ok(Json(FetchDatResponse {
            save_path: result.save_path,
            res_count: result.res_count,
            added_res_count: result.added_res_count,
        }))
    }

    /// 5ch の subject.txt（板のスレッド一覧）をインターネットから取得して UTF-8 で返す
    #[tool(annotations(read_only_hint = true, open_world_hint = true))]
    async fn fetch_subject(
        &self,
        params: Parameters<FetchSubjectToolParams>,
    ) -> Result<Json<FetchSubjectResponse>, String> {
        let p = &params.0;
        let result = fetch_subject::fetch_subject(
            &self.http_client,
            &fetch_subject::FetchSubjectParams {
                url: p.url.clone(),
                title_contains: p.title_contains.clone(),
            },
        )
        .await
        .map_err(|e| {
            warn!(?e, "fetch_subject failed");
            format!("{e:#}")
        })?;
        Ok(Json(FetchSubjectResponse {
            threads: result
                .threads
                .into_iter()
                .map(|t| SubjectThreadEntry {
                    thread_id: t.thread_id,
                    title: t.title,
                    res_count: t.res_count,
                })
                .collect(),
        }))
    }
}

#[tool_handler]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions("5ちゃんねるの dat ファイルの取得・読み取りを行う")
    }
}

async fn run_mcp_server(dat_dir: PathBuf, disable_body_limit: bool) {
    let http_client = match fetch_subject::build_client() {
        Ok(client) => client,
        Err(e) => {
            error!(?e, "failed to initialize HTTP client");
            return;
        }
    };

    let server =
        McpServer::new(dat_dir, disable_body_limit, http_client).serve(rmcp::transport::stdio());
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

    #[test]
    fn get_info_has_tools_capability() {
        let server = McpServer::new(PathBuf::new(), false, reqwest::Client::new());
        let info = server.get_info();
        assert!(
            info.capabilities.tools.is_some(),
            "ServerCapabilities should have tools capability"
        );
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
            let http_client = fetch_subject::build_client()?;
            let (server_transport, client_transport) = tokio::io::duplex(4096);
            let server_handle = tokio::spawn(async move {
                McpServer::new(dat_dir, false, http_client)
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
            let result = self.call_raw(tool, args).await?;

            if result.is_error.unwrap_or(false) {
                let text = result
                    .content
                    .first()
                    .and_then(|c| c.as_text())
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
                .and_then(|c| c.as_text())
                .map(|t| t.text.to_string())
                .unwrap_or_default();
            Ok(serde_json::from_str(&text)?)
        }

        async fn call_raw(
            &self,
            tool: &str,
            args: serde_json::Value,
        ) -> Fallible<rmcp::model::CallToolResult> {
            let tool_name = tool.to_string();
            Ok(self
                .client
                .call_tool(
                    CallToolRequestParams::new(tool_name)
                        .with_arguments(args.as_object().unwrap().clone()),
                )
                .await?)
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
    async fn mcp_read_posts_include_urls() -> Fallible<()> {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone()).await?;

        let parsed = ctx
            .call(
                "read_posts",
                json!({ "file": "630", "range": "1-1", "include_urls": true }),
            )
            .await?;
        let columns = parsed["columns"].as_array().unwrap();
        assert!(columns.iter().any(|c| c == "urls"));
        let row = &parsed["rows"][0];
        let urls_idx = columns.iter().position(|c| c == "urls").unwrap();
        let urls = row[urls_idx].as_array().unwrap();
        assert!(!urls.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn mcp_read_posts_no_urls_by_default() -> Fallible<()> {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone()).await?;

        let parsed = ctx
            .call("read_posts", json!({ "file": "630", "range": "1-1" }))
            .await?;
        let columns = parsed["columns"].as_array().unwrap();
        assert!(!columns.iter().any(|c| c == "urls"));
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

    /// Spawns a local `subject.txt` server. Never connects to 5ch.
    async fn spawn_subject_server(status: axum::http::StatusCode, body: Vec<u8>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let body = std::sync::Arc::new(body);
        let router = axum::Router::new().route(
            "/subject.txt",
            axum::routing::get(move || {
                let body = body.clone();
                async move {
                    (
                        status,
                        [(axum::http::header::CONTENT_TYPE, "text/plain")],
                        body.as_ref().clone(),
                    )
                }
            }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        format!("http://{addr}/subject.txt")
    }

    fn subject_body() -> Vec<u8> {
        let (bytes, _, _) = encoding_rs::SHIFT_JIS.encode(
            "1234567890.dat<>テストスレッド★630  (649)\n\
             1234567892.dat<>&#129402;絵文字テストスレッド&#129402;★631  (398)\n\
             0000000001.dat<>Test Thread Alpha  (427)\n",
        );
        bytes.into_owned()
    }

    #[tokio::test]
    async fn mcp_tools_list_has_fetch_subject() -> Fallible<()> {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone()).await?;

        let tools = ctx.client.list_all_tools().await?;
        let tool = tools
            .iter()
            .find(|t| t.name == "fetch_subject")
            .expect("fetch_subject should be listed");

        let properties = tool.input_schema.get("properties").unwrap();
        assert!(properties.get("url").is_some());
        assert!(properties.get("title_contains").is_some());
        let required = tool
            .input_schema
            .get("required")
            .unwrap()
            .as_array()
            .unwrap();
        assert!(required.iter().any(|v| v == "url"));
        assert!(!required.iter().any(|v| v == "title_contains"));

        let annotations = tool.annotations.as_ref().unwrap();
        assert_eq!(annotations.read_only_hint, Some(true));
        assert_eq!(annotations.open_world_hint, Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn mcp_fetch_subject() -> Fallible<()> {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone()).await?;
        let url = spawn_subject_server(axum::http::StatusCode::OK, subject_body()).await;

        let parsed = ctx.call("fetch_subject", json!({ "url": url })).await?;
        let threads = parsed["threads"].as_array().unwrap();
        assert_eq!(threads.len(), 3);
        assert_eq!(threads[0]["thread_id"], "1234567890");
        assert_eq!(threads[0]["title"], "テストスレッド★630");
        assert_eq!(threads[0]["res_count"], 649);
        assert_eq!(threads[1]["title"], "🥺絵文字テストスレッド🥺★631");
        assert_eq!(threads[2]["thread_id"], "0000000001");
        Ok(())
    }

    #[tokio::test]
    async fn mcp_fetch_subject_title_contains() -> Fallible<()> {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone()).await?;
        let url = spawn_subject_server(axum::http::StatusCode::OK, subject_body()).await;

        let parsed = ctx
            .call(
                "fetch_subject",
                json!({ "url": url, "title_contains": "thread ALPHA" }),
            )
            .await?;
        let threads = parsed["threads"].as_array().unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0]["title"], "Test Thread Alpha");
        Ok(())
    }

    #[tokio::test]
    async fn mcp_fetch_subject_invalid_url_is_error() -> Fallible<()> {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone()).await?;

        let result = ctx
            .call_raw(
                "fetch_subject",
                json!({ "url": "ftp://example.com/subject.txt" }),
            )
            .await?;
        assert_eq!(result.is_error, Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn mcp_fetch_subject_http_error_is_error() -> Fallible<()> {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone()).await?;
        let url = spawn_subject_server(axum::http::StatusCode::NOT_FOUND, Vec::new()).await;

        let result = ctx.call_raw("fetch_subject", json!({ "url": url })).await?;
        assert_eq!(result.is_error, Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn mcp_fetch_subject_parse_error_is_error() -> Fallible<()> {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone()).await?;
        let url = spawn_subject_server(
            axum::http::StatusCode::OK,
            b"<html><body>Not Found</body></html>".to_vec(),
        )
        .await;

        let result = ctx.call_raw("fetch_subject", json!({ "url": url })).await?;
        assert_eq!(result.is_error, Some(true));
        let text = result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.to_string())
            .unwrap_or_default();
        assert!(text.contains("1 行目"), "{text}");
        Ok(())
    }
}
