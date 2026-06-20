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

use clap::Parser;
use reqwest::header;
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use rmcp::schemars::{self, JsonSchema};
use rmcp::{Json, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use rust_myscript::prelude::*;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::Level;

/// Perplexity Search API endpoint.
const SEARCH_API_URL: &str = "https://api.perplexity.ai/search";

/// HTTP request timeout. Matches the default of the official Perplexity MCP server.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

/// Perplexity Search API MCP server
#[derive(Parser)]
struct Opt {
    /// API Key for Perplexity AI
    #[arg(long, env = "PERPLEXITY_API_KEY")]
    api_key: String,
}

/// Amount of content to extract from each page.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum SearchContextSize {
    Low,
    Medium,
    High,
}

/// Time-based recency filter for search results.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum SearchRecencyFilter {
    Hour,
    Day,
    Week,
    Month,
    Year,
}

#[derive(Deserialize, JsonSchema)]
struct SearchToolParams {
    /// 検索したい内容を表すクエリ。具体的なキーワードや質問文を指定する。
    /// 文脈・期間・正確な用語を含めるほど関連性の高い結果が得られる（曖昧な語は避ける）。
    query: String,

    /// 返す検索結果の最大件数。1〜20、未指定なら 10。
    /// 必要な件数だけにすると応答が速くなる（多いほど遅くなる）。
    #[serde(default)]
    max_results: Option<u8>,

    /// 各ページから抽出する本文トークンの上限（最大 1,000,000）。指定すると応答が長くなるため、
    /// 通常は未指定でよい。`search_context_size` とは同一リクエストで併用できない。
    /// Search API は定額課金（1,000 リクエストで $5）でトークン課金はなく、この値で費用は変わらない。
    #[serde(default)]
    max_tokens_per_page: Option<u32>,

    /// 全検索結果から取得する本文トークンの総量の上限（最大 1,000,000）。指定すると応答が長くなるため、
    /// 通常は未指定でよい。`search_context_size` とは同一リクエストで併用できない。
    /// これも費用には影響しない（Search API はリクエスト単位の定額課金）。
    #[serde(default)]
    max_tokens: Option<u32>,

    /// 検索対象を特定の国に絞る場合の ISO 3166-1 alpha-2 国コード（"US" / "JP" など）。
    /// 地域ニュースや地域固有の情報に有効。国を限定する必要がなければ未指定でよい。
    #[serde(default)]
    country: Option<String>,

    /// 検索結果を特定の言語に絞る ISO 639-1 言語コード（2 文字小文字、"en" / "ja" など）のリスト。
    /// 最大 20 件（公式ガイドの案内は 10 件）。大文字や無効なコードはエラーになる。限定しないなら空でよい。
    #[serde(default)]
    search_language_filter: Vec<String>,

    /// 検索対象を特定のドメインに絞るドメイン名（プロトコル無し、"nature.com" など）のリスト。最大 20 件。
    /// プレフィックス無しは許可リスト、先頭 "-" は除外リスト（両者は混在不可）。".gov" のような TLD やパス指定も可。
    /// ドメインを限定する必要がなければ空でよい。
    #[serde(default)]
    search_domain_filter: Vec<String>,

    /// 各ページから抽出する本文量。"low"（最小限）/ "medium"（バランス）/ "high"（詳細）のいずれか。未指定なら "high"。
    /// 軽量プレビューやトークン削減には "low"。迷う場合は未指定（"high"）でよい。
    /// `max_tokens` / `max_tokens_per_page` とは同一リクエストで併用できない。
    /// 料金は定額のためこの設定で費用は変わらない。
    #[serde(default)]
    search_context_size: Option<SearchContextSize>,

    /// 「更新日」がこの日付より後の結果のみを返す（MM/DD/YYYY 形式、先頭ゼロ省略可、例 "3/1/2025"）。
    /// 期間を限定する必要がなければ未指定でよい。
    #[serde(default)]
    last_updated_after_filter: Option<String>,

    /// 「更新日」がこの日付より前の結果のみを返す（MM/DD/YYYY 形式、例 "3/5/2025"）。
    /// 期間を限定する必要がなければ未指定でよい。
    #[serde(default)]
    last_updated_before_filter: Option<String>,

    /// 「公開日」がこの日付より後の結果のみを返す（MM/DD/YYYY 形式、先頭ゼロ省略可、例 "3/1/2025"）。
    /// 期間を限定する必要がなければ未指定でよい。
    #[serde(default)]
    search_after_date_filter: Option<String>,

    /// 「公開日」がこの日付より前の結果のみを返す（MM/DD/YYYY 形式、例 "3/5/2025"）。
    /// 期間を限定する必要がなければ未指定でよい。
    #[serde(default)]
    search_before_date_filter: Option<String>,

    /// 公開時期の新しさで絞り込む。"hour"（過去1時間）/ "day"（過去24時間）/ "week"（過去7日）/
    /// "month"（過去30日）/ "year"（過去365日）のいずれか。期間を限定しないなら未指定でよい。
    /// 日付フィルタ（search_*_date_filter / last_updated_*_filter）とは併用できない。
    #[serde(default)]
    search_recency_filter: Option<SearchRecencyFilter>,
}

/// Request body for the Perplexity Search API (`POST https://api.perplexity.ai/search`).
///
/// See <https://docs.perplexity.ai/api-reference/search-post>.
///
/// Fields left unset are omitted from the serialized body.
#[derive(Debug, Serialize)]
struct SearchPostRequest {
    /// Search query.
    query: String,
    /// Maximum number of results to return (1-20, default 10).
    #[serde(skip_serializing_if = "Option::is_none")]
    max_results: Option<u8>,
    /// Maximum webpage content tokens to extract per page (up to 1,000,000).
    /// Cannot be combined with `search_context_size`.
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens_per_page: Option<u32>,
    /// Maximum total webpage content tokens to return across all results (up to 1,000,000).
    /// Cannot be combined with `search_context_size`.
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    /// Region filter (ISO 3166-1 alpha-2).
    #[serde(skip_serializing_if = "Option::is_none")]
    country: Option<String>,
    /// Language filter (ISO 639-1, two lowercase letters; OpenAPI allows up to 20 entries).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    search_language_filter: Vec<String>,
    /// Domain filter (up to 20 entries). Bare domain = allowlist, leading `-` = denylist.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    search_domain_filter: Vec<String>,
    /// Amount of content to extract from each page.
    /// Cannot be combined with `max_tokens` or `max_tokens_per_page`.
    #[serde(skip_serializing_if = "Option::is_none")]
    search_context_size: Option<SearchContextSize>,
    /// Return results updated after this date (MM/DD/YYYY).
    #[serde(skip_serializing_if = "Option::is_none")]
    last_updated_after_filter: Option<String>,
    /// Return results updated before this date (MM/DD/YYYY).
    #[serde(skip_serializing_if = "Option::is_none")]
    last_updated_before_filter: Option<String>,
    /// Return results published after this date (MM/DD/YYYY).
    #[serde(skip_serializing_if = "Option::is_none")]
    search_after_date_filter: Option<String>,
    /// Return results published before this date (MM/DD/YYYY).
    #[serde(skip_serializing_if = "Option::is_none")]
    search_before_date_filter: Option<String>,
    /// Filter by publication recency. Cannot be combined with the explicit date filters above.
    #[serde(skip_serializing_if = "Option::is_none")]
    search_recency_filter: Option<SearchRecencyFilter>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct SearchResponse {
    /// 検索結果の一覧
    results: Vec<SearchResultEntry>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct SearchResultEntry {
    /// ページのタイトル
    title: String,

    /// ページの URL
    url: String,

    /// 抜粋（取得できた場合のみ）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    snippet: Option<String>,

    /// 公開日（取得できた場合のみ）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    date: Option<String>,

    /// 最終更新日（取得できた場合のみ）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_updated: Option<String>,
}

struct McpServer {
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
        let response = execute_search(&self.client, &self.api_key, &body)
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
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
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

/// Sends the request to the Perplexity Search API
/// (`POST https://api.perplexity.ai/search`) and parses the response.
///
/// See <https://docs.perplexity.ai/api-reference/search-post>.
async fn execute_search(
    client: &reqwest::Client,
    api_key: &str,
    body: &SearchPostRequest,
) -> Fallible<SearchResponse> {
    let res = client
        .post(SEARCH_API_URL)
        .header(header::ACCEPT, "application/json")
        .bearer_auth(api_key)
        .json(body)
        .send()
        .await
        .context("Search API へのリクエストに失敗しました")?;

    let status = res.status();
    let res_text = res
        .text()
        .await
        .context("Search API レスポンスの読み取りに失敗しました")?;

    if !status.is_success() {
        bail!("Search API がエラーを返しました ({status}): {res_text}");
    }

    parse_search_response(&res_text)
}

/// Parses the Search API response body. Unknown fields (e.g. `id`, `server_time`) are ignored.
fn parse_search_response(text: &str) -> Fallible<SearchResponse> {
    serde_json::from_str::<SearchResponse>(text)
        .with_context(|| format!("Search API レスポンスの解析に失敗しました: {text}"))
}

fn build_client() -> Fallible<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(concat!(
            env!("CARGO_PKG_NAME"),
            " (https://github.com/sukawasatoru/rust-myscript/)"
        ))
        .build()
        .context("HTTP クライアントの初期化に失敗しました")
}

async fn run_mcp_server(api_key: String) {
    let client = match build_client() {
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

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_writer(std::io::stderr)
        .init();

    let opt = Opt::parse();
    run_mcp_server(opt.api_key).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use rmcp::service::RunningService;
    use rmcp::{ClientHandler, RoleClient, ServiceExt};
    use serde_json::json;

    #[test]
    fn struct_opt() {
        Opt::command().debug_assert();
    }

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

    #[test]
    fn parse_search_response_ignores_unknown_fields() {
        let parsed = parse_search_response(RES_TEXT).unwrap();
        assert_eq!(parsed.results.len(), 2);
        assert_eq!(parsed.results[0].title, "Example Title");
        assert_eq!(parsed.results[0].url, "https://example.com/");
        assert_eq!(
            parsed.results[0].snippet.as_deref(),
            Some("an example snippet")
        );
        assert_eq!(parsed.results[0].date.as_deref(), Some("2026-06-20"));
    }

    #[test]
    fn parse_search_response_minimal_entry() {
        let parsed =
            parse_search_response(r#"{"results":[{"title":"t","url":"https://e/"}]}"#).unwrap();
        assert_eq!(parsed.results.len(), 1);
        assert_eq!(parsed.results[0].snippet, None);
        assert_eq!(parsed.results[0].date, None);
        assert_eq!(parsed.results[0].last_updated, None);
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

    const RES_TEXT: &str = r#"
{
  "results": [
    {
      "title": "Example Title",
      "url": "https://example.com/",
      "snippet": "an example snippet",
      "date": "2026-06-20",
      "last_updated": "2026-06-20T00:00:00Z"
    },
    {
      "title": "Second Title",
      "url": "https://example.org/"
    }
  ],
  "id": "abc-123",
  "server_time": "2026-06-20T02:28:00Z"
}
"#;
}
