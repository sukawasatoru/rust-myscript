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

//! Perplexity Search API requests, HTTP client policy, and response decoding.

use crate::dto::{SearchContextSize, SearchRecencyFilter, SearchResponse};
use reqwest::header;
use rust_myscript::prelude::*;
use serde::Serialize;
use std::time::Duration;

/// Perplexity Search API endpoint.
const SEARCH_API_URL: &str = "https://api.perplexity.ai/search";

/// HTTP request timeout. Matches the default of the official Perplexity MCP server.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

/// Request body for the Perplexity Search API (`POST https://api.perplexity.ai/search`).
///
/// See <https://docs.perplexity.ai/api-reference/search-post>.
///
/// Fields left unset are omitted from the serialized body.
#[derive(Debug, Serialize)]
pub(crate) struct SearchPostRequest {
    /// Search query.
    pub(crate) query: String,
    /// Maximum number of results to return (1-20, default 10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_results: Option<u8>,
    /// Maximum webpage content tokens to extract per page (up to 1,000,000).
    /// Cannot be combined with `search_context_size`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_tokens_per_page: Option<u32>,
    /// Maximum total webpage content tokens to return across all results (up to 1,000,000).
    /// Cannot be combined with `search_context_size`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_tokens: Option<u32>,
    /// Region filter (ISO 3166-1 alpha-2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) country: Option<String>,
    /// Language filter (ISO 639-1, two lowercase letters; OpenAPI allows up to 20 entries).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) search_language_filter: Vec<String>,
    /// Domain filter (up to 20 entries). Bare domain = allowlist, leading `-` = denylist.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) search_domain_filter: Vec<String>,
    /// Amount of content to extract from each page.
    /// Cannot be combined with `max_tokens` or `max_tokens_per_page`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) search_context_size: Option<SearchContextSize>,
    /// Return results updated after this date (MM/DD/YYYY).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_updated_after_filter: Option<String>,
    /// Return results updated before this date (MM/DD/YYYY).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_updated_before_filter: Option<String>,
    /// Return results published after this date (MM/DD/YYYY).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) search_after_date_filter: Option<String>,
    /// Return results published before this date (MM/DD/YYYY).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) search_before_date_filter: Option<String>,
    /// Filter by publication recency. Cannot be combined with the explicit date filters above.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) search_recency_filter: Option<SearchRecencyFilter>,
}

/// Sends the request to the Perplexity Search API
/// (`POST https://api.perplexity.ai/search`) and parses the response.
///
/// See <https://docs.perplexity.ai/api-reference/search-post>.
pub(crate) async fn execute_search(
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

pub(crate) fn build_client() -> Fallible<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(concat!(
            env!("CARGO_PKG_NAME"),
            " (https://github.com/sukawasatoru/rust-myscript/)"
        ))
        .build()
        .context("Failed to initialize the HTTP client")
}

#[cfg(test)]
mod tests {
    use super::*;

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
