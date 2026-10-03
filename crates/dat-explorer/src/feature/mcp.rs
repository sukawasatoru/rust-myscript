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

use crate::application::thread_service::ThreadService;
use crate::dto::mcp::*;
use crate::model::fetch::{FetchDatParams, FetchSubjectParams};
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{Json, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use rust_myscript::prelude::*;
use serde_json::json;

mod read_posts;
mod response;
mod search_posts;

struct McpServer {
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
    service: ThreadService,
    disable_body_limit: bool,
}

#[tool_router]
impl McpServer {
    fn new(service: ThreadService, disable_body_limit: bool) -> Self {
        Self {
            tool_router: Self::tool_router(),
            service,
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
            &self.service,
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

    /// 保存済み dat のスレッド一覧を作成日時降順で返す
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn list_threads(&self) -> Result<Json<ListThreadsResponse>, String> {
        let threads = self.service.list_threads().map_err(|e| format!("{e:#}"))?;
        Ok(Json(ListThreadsResponse {
            threads: threads
                .into_iter()
                .map(|entry| LocalThreadEntry {
                    file: entry.file,
                    title: entry.title,
                    post_count: entry.post_count,
                    created_at: entry.created_at,
                })
                .collect(),
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
            &self.service,
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
        let result = self
            .service
            .fetch_dat(&FetchDatParams {
                url: p.url.clone(),
                save_path: p.save_path.clone(),
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
        let result = self
            .service
            .fetch_subject(&FetchSubjectParams {
                url: p.url.clone(),
                title_contains: p.title_contains.clone(),
            })
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
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions("5ちゃんねるの dat ファイルの取得・読み取りを行う")
    }
}

pub async fn run(service: ThreadService, disable_body_limit: bool) {
    let server = McpServer::new(service, disable_body_limit).serve(rmcp::transport::stdio());
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
    use crate::data::five_ch::subject as fetch_subject;
    use rmcp::model::CallToolRequestParams;
    use rmcp::service::RunningService;
    use rmcp::{ClientHandler, RoleClient, ServiceExt};
    use serde_json::json;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn get_info_has_tools_capability() {
        let service = ThreadService::new(PathBuf::new(), reqwest::Client::new());
        let server = McpServer::new(service, false);
        let info = server.get_info();
        assert!(
            info.capabilities.tools.is_some(),
            "ServerCapabilities should have tools capability"
        );
    }

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
            Self::with_body_limit(dat_dir, false).await
        }

        async fn with_body_limit(dat_dir: PathBuf, disable_body_limit: bool) -> Fallible<Self> {
            let http_client = fetch_subject::build_client()?;
            let (server_transport, client_transport) = tokio::io::duplex(4096);
            let server_handle = tokio::spawn(async move {
                McpServer::new(ThreadService::new(dat_dir, http_client), disable_body_limit)
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
    async fn mcp_read_posts() {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone())
            .await
            .unwrap();

        let parsed = ctx
            .call("read_posts", json!({ "file": "630", "range": "1-2" }))
            .await
            .unwrap();
        assert_eq!(parsed["rows"].as_array().unwrap().len(), 2);
        assert_eq!(
            parsed["columns"],
            json!(["res_num", "datetime", "body", "ref_count"])
        );
        assert_eq!(parsed["file_info"]["thread_num"], 630);
        assert!(parsed["file_info"]["date_range"].is_string());
    }

    #[tokio::test]
    async fn mcp_read_posts_include_urls() {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone())
            .await
            .unwrap();

        let parsed = ctx
            .call(
                "read_posts",
                json!({ "file": "630", "range": "1-1", "include_urls": true }),
            )
            .await
            .unwrap();
        let columns = parsed["columns"].as_array().unwrap();
        assert!(columns.iter().any(|c| c == "urls"));
        let row = &parsed["rows"][0];
        let urls_idx = columns.iter().position(|c| c == "urls").unwrap();
        let urls = row[urls_idx].as_array().unwrap();
        assert!(!urls.is_empty());
    }

    #[tokio::test]
    async fn mcp_read_posts_no_urls_by_default() {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone())
            .await
            .unwrap();

        let parsed = ctx
            .call("read_posts", json!({ "file": "630", "range": "1-1" }))
            .await
            .unwrap();
        let columns = parsed["columns"].as_array().unwrap();
        assert!(!columns.iter().any(|c| c == "urls"));
    }

    #[tokio::test]
    async fn mcp_search_posts_with_keywords() {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone())
            .await
            .unwrap();

        let parsed = ctx
            .call("search_posts", json!({ "keywords": ["Tool v2\\.5"] }))
            .await
            .unwrap();
        assert_eq!(parsed["total_hits"], 1);
        assert_eq!(
            parsed["columns"],
            json!(["file", "res_num", "datetime", "body", "urls", "ref_count"])
        );
    }

    #[tokio::test]
    async fn mcp_search_posts_no_keywords_error() {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone())
            .await
            .unwrap();

        let result = ctx.call("search_posts", json!({})).await;
        assert!(result.is_err());
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
    async fn mcp_tools_list_has_fetch_subject() {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone())
            .await
            .unwrap();

        let tools = ctx.client.list_all_tools().await.unwrap();
        let mut names: Vec<_> = tools.iter().map(|t| t.name.as_ref()).collect();
        names.sort();
        assert_eq!(
            names,
            [
                "fetch_dat",
                "fetch_subject",
                "list_threads",
                "read_posts",
                "search_posts"
            ]
        );
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
    }

    #[tokio::test]
    async fn mcp_fetch_subject() {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone())
            .await
            .unwrap();
        let url = spawn_subject_server(axum::http::StatusCode::OK, subject_body()).await;

        let parsed = ctx
            .call("fetch_subject", json!({ "url": url }))
            .await
            .unwrap();
        let threads = parsed["threads"].as_array().unwrap();
        assert_eq!(threads.len(), 3);
        assert_eq!(threads[0]["thread_id"], "1234567890");
        assert_eq!(threads[0]["title"], "テストスレッド★630");
        assert_eq!(threads[0]["res_count"], 649);
        assert_eq!(threads[1]["title"], "🥺絵文字テストスレッド🥺★631");
        assert_eq!(threads[2]["thread_id"], "0000000001");
    }

    #[tokio::test]
    async fn mcp_fetch_subject_title_contains() {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone())
            .await
            .unwrap();
        let url = spawn_subject_server(axum::http::StatusCode::OK, subject_body()).await;

        let parsed = ctx
            .call(
                "fetch_subject",
                json!({ "url": url, "title_contains": "thread ALPHA" }),
            )
            .await
            .unwrap();
        let threads = parsed["threads"].as_array().unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0]["title"], "Test Thread Alpha");
    }

    #[tokio::test]
    async fn mcp_fetch_subject_invalid_url_is_error() {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone())
            .await
            .unwrap();

        let result = ctx
            .call_raw(
                "fetch_subject",
                json!({ "url": "ftp://example.com/subject.txt" }),
            )
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn mcp_fetch_subject_http_error_is_error() {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone())
            .await
            .unwrap();
        let url = spawn_subject_server(axum::http::StatusCode::NOT_FOUND, Vec::new()).await;

        let result = ctx
            .call_raw("fetch_subject", json!({ "url": url }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn mcp_fetch_subject_parse_error_is_error() {
        let test_dirs = create_test_dirs();
        let ctx = McpTestContext::new(test_dirs.dat_dir.clone())
            .await
            .unwrap();
        let url = spawn_subject_server(
            axum::http::StatusCode::OK,
            b"<html><body>Not Found</body></html>".to_vec(),
        )
        .await;

        let result = ctx
            .call_raw("fetch_subject", json!({ "url": url }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        let text = result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.to_string())
            .unwrap_or_default();
        assert!(text.contains("1 行目"), "{text}");
    }

    #[tokio::test]
    async fn mcp_read_posts_preserves_column_order_and_specific_numbers() {
        let dirs = create_test_dirs();
        let ctx = McpTestContext::new(dirs.dat_dir.clone()).await.unwrap();
        let result = ctx
            .call(
                "read_posts",
                json!({
                    "file": "630", "range": "1-1", "res_nums": [3, 2, 2],
                    "include_name": true, "include_id": true, "include_urls": true
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            result["columns"],
            json!([
                "res_num",
                "name",
                "datetime",
                "id",
                "body",
                "ref_count",
                "urls"
            ])
        );
        let rows = result["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            json!([
                2,
                "名無し",
                "2026/03/13(金) 11:00:00.00",
                "test0002",
                "Tool v2.5すごい\n>>1 これは便利",
                0,
                []
            ])
        );
        assert_eq!(rows[1][0], 3);
        assert!(result.get("omitted_count").is_none());
    }

    #[tokio::test]
    async fn mcp_budget_and_disable_body_limit_apply_to_reads_and_searches() {
        let dirs = create_test_dirs();
        let body = "本文".repeat(30000);
        let text = format!(
            "名無し<><>2026/01/01 ID:a<>{body}<>タイトル\n名無し<><>2026/01/02 ID:b<>本文その2<>"
        );
        std::fs::write(dirs.dat_dir.join("board_700_123.dat"), text).unwrap();
        for disabled in [false, true] {
            let ctx = McpTestContext::with_body_limit(dirs.dat_dir.clone(), disabled)
                .await
                .unwrap();
            for (tool, mut args) in [
                ("read_posts", json!({"file": "700"})),
                (
                    "search_posts",
                    json!({"files": ["700"], "keywords": ["本文"]}),
                ),
            ] {
                let result = ctx.call(tool, args.clone()).await.unwrap();
                assert_eq!(
                    result["rows"].as_array().unwrap().len(),
                    if disabled { 2 } else { 1 }
                );
                if disabled {
                    assert!(result.get("omitted_count").is_none());
                } else {
                    assert_eq!(result["omitted_count"], 1);
                }
                // An explicit user budget still applies even when the safety cap is disabled.
                args["max_body_chars"] = json!(1);
                let limited = ctx.call(tool, args).await.unwrap();
                assert_eq!(limited["rows"].as_array().unwrap().len(), 1);
                assert_eq!(limited["omitted_count"], 1);
                if tool == "search_posts" {
                    assert_eq!(limited["total_hits"], 2);
                }
            }
        }
    }

    #[tokio::test]
    async fn mcp_fetch_dat_invalid_url_is_tool_error() {
        let dirs = create_test_dirs();
        let ctx = McpTestContext::new(dirs.dat_dir.clone()).await.unwrap();
        let result = ctx
            .call_raw(
                "fetch_dat",
                json!({
                    "url": "https://example.com/invalid", "save_path": "new.dat"
                }),
            )
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert!(!dirs.dat_dir.join("new.dat").exists());
    }
}
