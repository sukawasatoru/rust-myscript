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

//! MCP input/output types and shared Search API enums.
//! Search results are shared by the API decoder and MCP structured output.

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};

/// Amount of content to extract from each page.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SearchContextSize {
    Low,
    Medium,
    High,
}

/// Time-based recency filter for search results.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SearchRecencyFilter {
    Hour,
    Day,
    Week,
    Month,
    Year,
}

#[derive(Deserialize, JsonSchema)]
pub(crate) struct SearchToolParams {
    /// 検索したい内容を表すクエリ。具体的なキーワードや質問文を指定する。
    /// 文脈・期間・正確な用語を含めるほど関連性の高い結果が得られる（曖昧な語は避ける）。
    pub(crate) query: String,

    /// 返す検索結果の最大件数。1〜20、未指定なら 10。
    /// 必要な件数だけにすると応答が速くなる（多いほど遅くなる）。
    /// 通常は未指定でよい。
    #[serde(default)]
    pub(crate) max_results: Option<u8>,

    /// 各ページから抽出する本文トークンの上限（最大 1,000,000）。指定すると応答が長くなるため、
    /// 通常は未指定でよい。`search_context_size` とは同一リクエストで併用できない。
    /// Search API は定額課金（1,000 リクエストで $5）でトークン課金はなく、この値で費用は変わらない。
    #[serde(default)]
    pub(crate) max_tokens_per_page: Option<u32>,

    /// 全検索結果から取得する本文トークンの総量の上限（最大 1,000,000）。指定すると応答が長くなるため、
    /// 通常は未指定でよい。`search_context_size` とは同一リクエストで併用できない。
    /// これも費用には影響しない（Search API はリクエスト単位の定額課金）。
    #[serde(default)]
    pub(crate) max_tokens: Option<u32>,

    /// 検索対象を特定の国に絞る場合の ISO 3166-1 alpha-2 国コード（"US" / "JP" など）。
    /// 地域ニュースや地域固有の情報に有効。国を限定する必要がなければ未指定でよい。
    #[serde(default)]
    pub(crate) country: Option<String>,

    /// 検索結果を特定の言語に絞る ISO 639-1 言語コード（2 文字小文字、"en" / "ja" など）のリスト。
    /// 最大 20 件（公式ガイドの案内は 10 件）。大文字や無効なコードはエラーになる。限定しないなら空でよい。
    #[serde(default)]
    pub(crate) search_language_filter: Vec<String>,

    /// 検索対象を特定のドメインに絞るドメイン名（プロトコル無し、"nature.com" など）のリスト。最大 20 件。
    /// プレフィックス無しは許可リスト、先頭 "-" は除外リスト（両者は混在不可）。".gov" のような TLD やパス指定も可。
    /// ドメインを限定する必要がなければ空でよい。
    #[serde(default)]
    pub(crate) search_domain_filter: Vec<String>,

    /// 各ページから抽出する本文量。"low"（最小限）/ "medium"（バランス）/ "high"（詳細）のいずれか。未指定なら "high"。
    /// 軽量プレビューやトークン削減には "low"。迷う場合は未指定（"high"）でよい。
    /// `max_tokens` / `max_tokens_per_page` とは同一リクエストで併用できない。
    /// 料金は定額のためこの設定で費用は変わらない。
    #[serde(default)]
    pub(crate) search_context_size: Option<SearchContextSize>,

    /// 「更新日」がこの日付より後の結果のみを返す（MM/DD/YYYY 形式、先頭ゼロ省略可、例 "3/1/2025"）。
    /// 期間を限定する必要がなければ未指定でよい。
    #[serde(default)]
    pub(crate) last_updated_after_filter: Option<String>,

    /// 「更新日」がこの日付より前の結果のみを返す（MM/DD/YYYY 形式、例 "3/5/2025"）。
    /// 期間を限定する必要がなければ未指定でよい。
    #[serde(default)]
    pub(crate) last_updated_before_filter: Option<String>,

    /// 「公開日」がこの日付より後の結果のみを返す（MM/DD/YYYY 形式、先頭ゼロ省略可、例 "3/1/2025"）。
    /// 期間を限定する必要がなければ未指定でよい。
    #[serde(default)]
    pub(crate) search_after_date_filter: Option<String>,

    /// 「公開日」がこの日付より前の結果のみを返す（MM/DD/YYYY 形式、例 "3/5/2025"）。
    /// 期間を限定する必要がなければ未指定でよい。
    #[serde(default)]
    pub(crate) search_before_date_filter: Option<String>,

    /// 公開時期の新しさで絞り込む。"hour"（過去1時間）/ "day"（過去24時間）/ "week"（過去7日）/
    /// "month"（過去30日）/ "year"（過去365日）のいずれか。期間を限定しないなら未指定でよい。
    /// 日付フィルタ（search_*_date_filter / last_updated_*_filter）とは併用できない。
    #[serde(default)]
    pub(crate) search_recency_filter: Option<SearchRecencyFilter>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct SearchResponse {
    /// 検索結果の一覧
    pub(crate) results: Vec<SearchResultEntry>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct SearchResultEntry {
    /// ページのタイトル
    pub(crate) title: String,

    /// ページの URL
    pub(crate) url: String,

    /// 抜粋（取得できた場合のみ）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) snippet: Option<String>,

    /// 公開日（取得できた場合のみ）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) date: Option<String>,

    /// 最終更新日（取得できた場合のみ）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_updated: Option<String>,
}
