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

//! MCP tool routing, request conversion, and stdio serving.
//! The caller supplies a resolved API key; this module does not open credential storage.

use crate::data::perplexity::{self, SearchPostRequest};
use crate::dto::{SearchResponse, SearchToolParams};
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{Json, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use rust_myscript::prelude::*;

struct McpServer {
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
    api_key: String,
    client: reqwest::Client,
}

#[tool_router]
impl McpServer {
    fn new(api_key: String, client: reqwest::Client) -> Self {
        Self {
            tool_router: Self::tool_router(),
            api_key,
            client,
        }
    }

    /// 自然言語風クエリでリアルタイム Web 検索し、構造化された検索結果を返す
    #[tool(annotations(read_only_hint = true, open_world_hint = true))]
    async fn search(
        &self,
        params: Parameters<SearchToolParams>,
    ) -> Result<Json<SearchResponse>, String> {
        let body = build_request_body(&params.0);
        let response = perplexity::execute_search(&self.client, &self.api_key, &body)
            .await
            .map_err(|e| {
                warn!(?e, "search failed");
                e.to_string()
            })?;
        Ok(Json(response))
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
            .with_instructions("Perplexity API でリアルタイム Web 検索を行う")
    }
}

/// Builds the request body for the Perplexity Search API
/// (`POST https://api.perplexity.ai/search`).
///
/// See <https://docs.perplexity.ai/api-reference/search-post>.
fn build_request_body(params: &SearchToolParams) -> SearchPostRequest {
    SearchPostRequest {
        query: params.query.clone(),
        max_results: params.max_results,
        max_tokens_per_page: params.max_tokens_per_page,
        max_tokens: params.max_tokens,
        country: params.country.clone(),
        search_language_filter: params.search_language_filter.clone(),
        search_domain_filter: params.search_domain_filter.clone(),
        search_context_size: params.search_context_size,
        last_updated_after_filter: params.last_updated_after_filter.clone(),
        last_updated_before_filter: params.last_updated_before_filter.clone(),
        search_after_date_filter: params.search_after_date_filter.clone(),
        search_before_date_filter: params.search_before_date_filter.clone(),
        search_recency_filter: params.search_recency_filter,
    }
}

/// Run the stdio server, reporting transport and initialization failures to the log.
pub async fn run(api_key: String) {
    let client = match perplexity::build_client() {
        Ok(client) => client,
        Err(e) => {
            error!(?e, "failed to build HTTP client");
            return;
        }
    };

    let server = McpServer::new(api_key, client).serve(rmcp::transport::stdio());
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
    use crate::dto::{SearchContextSize, SearchRecencyFilter};
    use rmcp::service::RunningService;
    use rmcp::{ClientHandler, RoleClient, ServiceExt};
    use serde_json::json;

    #[test]
    fn get_info_has_tools_capability() {
        let server = McpServer::new(String::new(), reqwest::Client::new());
        let info = server.get_info();
        assert!(
            info.capabilities.tools.is_some(),
            "ServerCapabilities should have tools capability"
        );
    }

    #[test]
    fn build_request_body_minimal() {
        let params = SearchToolParams {
            query: "rust language".to_string(),
            max_results: None,
            max_tokens_per_page: None,
            max_tokens: None,
            country: None,
            search_language_filter: vec![],
            search_domain_filter: vec![],
            search_context_size: None,
            last_updated_after_filter: None,
            last_updated_before_filter: None,
            search_after_date_filter: None,
            search_before_date_filter: None,
            search_recency_filter: None,
        };
        assert_eq!(
            serde_json::to_value(build_request_body(&params)).unwrap(),
            json!({ "query": "rust language" })
        );
    }

    #[test]
    fn build_request_body_full() {
        let params = SearchToolParams {
            query: "rust language".to_string(),
            max_results: Some(5),
            max_tokens_per_page: Some(512),
            max_tokens: Some(2048),
            country: Some("US".to_string()),
            search_language_filter: vec!["en".to_string(), "ja".to_string()],
            search_domain_filter: vec!["example.com".to_string()],
            search_context_size: Some(SearchContextSize::High),
            last_updated_after_filter: Some("01/01/2025".to_string()),
            last_updated_before_filter: Some("12/31/2025".to_string()),
            search_after_date_filter: Some("01/01/2024".to_string()),
            search_before_date_filter: Some("12/31/2024".to_string()),
            search_recency_filter: Some(SearchRecencyFilter::Week),
        };
        assert_eq!(
            serde_json::to_value(build_request_body(&params)).unwrap(),
            json!({
                "query": "rust language",
                "max_results": 5,
                "max_tokens_per_page": 512,
                "max_tokens": 2048,
                "country": "US",
                "search_language_filter": ["en", "ja"],
                "search_domain_filter": ["example.com"],
                "search_context_size": "high",
                "last_updated_after_filter": "01/01/2025",
                "last_updated_before_filter": "12/31/2025",
                "search_after_date_filter": "01/01/2024",
                "search_before_date_filter": "12/31/2024",
                "search_recency_filter": "week",
            })
        );
    }

    #[derive(Debug, Clone, Default)]
    struct DummyClientHandler;
    impl ClientHandler for DummyClientHandler {}

    struct McpTestContext {
        client: RunningService<RoleClient, DummyClientHandler>,
        server_handle: tokio::task::JoinHandle<anyhow::Result<()>>,
    }

    impl McpTestContext {
        async fn new() -> Fallible<Self> {
            let (server_transport, client_transport) = tokio::io::duplex(4096);
            let server_handle = tokio::spawn(async move {
                McpServer::new("dummy-api-key".to_string(), reqwest::Client::new())
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
    }

    impl Drop for McpTestContext {
        fn drop(&mut self) {
            self.server_handle.abort();
            self.client.cancellation_token().cancel();
        }
    }

    #[tokio::test]
    async fn mcp_exposes_search_tool() {
        let ctx = McpTestContext::new().await.unwrap();
        let tools = ctx.client.list_all_tools().await.unwrap();
        assert!(
            tools.iter().any(|t| t.name.as_ref() == "search"),
            "server should expose a `search` tool"
        );
    }
}
