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

//! Wire types shared by the MCP server and clients. Keep schemas independent of domain models.

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, JsonSchema)]
pub struct ReadPostsToolParams {
    /// ファイル指定（スレ番号 "630" またはファイル名）
    pub file: String,
    /// レス番号の範囲（例: "1-100", "900-", "-50"）。res_nums と排他
    #[serde(default)]
    pub range: Option<String>,
    /// 特定のレス番号をリストで指定（例: [86, 87, 99]）。range より優先
    #[serde(default)]
    pub res_nums: Vec<usize>,
    /// 各レス本文の最大文字数。超過分は切り詰める。0 = 制限なし（デフォルト）
    #[serde(default)]
    pub max_body_chars: usize,
    /// true の場合 name カラムを含める（デフォルト: false）
    #[serde(default)]
    pub include_name: bool,
    /// true の場合 id カラムを含める（デフォルト: false）
    #[serde(default)]
    pub include_id: bool,
    /// true の場合 urls カラムを含める（デフォルト: false）
    #[serde(default)]
    pub include_urls: bool,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct ReadPostsResponse {
    pub file_info: FileInfoEntry,
    /// カラム名の一覧: ["res_num", "name", "datetime", "id", "body", "ref_count", "urls"] (name, id, urls は引数による)
    pub columns: Vec<String>,
    /// 各レスの値を columns の順に並べた配列
    pub rows: Vec<Vec<serde_json::Value>>,
    /// max_body_chars 超過により省略されたレス数
    #[serde(default, skip_serializing_if = "is_zero")]
    pub omitted_count: usize,
}

#[derive(Deserialize, JsonSchema)]
pub struct SearchPostsToolParams {
    /// 検索キーワード（正規表現対応）
    #[serde(default)]
    pub keywords: Vec<String>,
    /// 対象ファイル（スレ番号）。空の場合は全ファイル
    #[serde(default)]
    pub files: Vec<String>,
    /// レス番号の範囲
    #[serde(default)]
    pub range: Option<String>,
    /// 投稿者 ID でフィルタ（部分一致）。keywords なしでも使用可能
    #[serde(default)]
    pub ids: Vec<String>,
    /// ヒット本文の合計文字数の目安上限。超えたレスまで含めて打ち切る。0 = 制限なし（デフォルト）
    #[serde(default)]
    pub max_body_chars: usize,
    /// true の場合 id カラムを含める（デフォルト: false）
    #[serde(default)]
    pub include_id: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct SearchPostsResponse {
    pub total_hits: usize,
    pub searched_files: Vec<String>,
    /// カラム名の一覧: ["file", "res_num", "datetime", "id", "body", "urls", "ref_count"] (id は引数による)
    pub columns: Vec<String>,
    /// 各ヒットの値を columns の順に並べた配列
    pub rows: Vec<Vec<serde_json::Value>>,
    /// max_body_chars 超過により省略されたヒット数
    #[serde(default, skip_serializing_if = "is_zero")]
    pub omitted_count: usize,
}

#[derive(Deserialize, JsonSchema)]
pub struct FetchDatToolParams {
    /// スレッドの URL。以下の2形式に対応する。どちらの形式でも、まず dat 直接取得を試み、
    /// 404（dat落ち）の場合は自動的に read.cgi 経由（HTML取得・dat変換）に切り替える。
    ///
    /// 形式1 - dat URL（現行スレ・dat落ちスレ共通）:
    ///   "https://{server}.5ch.io/{board}/dat/{thread_id}.dat"
    ///
    /// 形式2 - read.cgi URL:
    ///   "https://{server}.5ch.io/test/read.cgi/{board}/{thread_id}/"
    pub url: String,

    /// 保存先ファイルパス。絶対パスまたは相対パスで指定する。
    /// 相対パスの場合は CLI 引数で指定した dat_dir を基準に解決する。
    /// 既存ファイルがある場合は上書き保存し、増加レス数を added_res_count で返す。
    /// 例（絶対パス）: "/path/to/dir/PREFIX_635_1234567890.dat"
    /// 例（相対パス）: "PREFIX_635_1234567890.dat"
    pub save_path: String,
}

#[derive(Serialize, JsonSchema)]
pub struct FetchDatResponse {
    /// 実際に保存したファイルの絶対パス
    pub save_path: String,
    /// 保存した dat のレス数（空行を除く行数）
    pub res_count: usize,
    /// 既存ファイルからの増加レス数。既存ファイルがなければ null、更新がなければ 0
    pub added_res_count: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
pub struct FetchSubjectToolParams {
    /// subject.txt の URL。http または https のみ指定できる。
    /// userinfo は指定可能。fragment は除去し、query は保持する。
    /// 例: "https://fate.5ch.io/liveuranus/subject.txt"
    pub url: String,

    /// 指定した場合、スレッドタイトルに含まれるものだけを返す。
    /// NCR 復元後のタイトルへの大小文字無視の部分一致。未指定または空文字は全件を返す。
    #[serde(default)]
    pub title_contains: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct FetchSubjectResponse {
    /// subject.txt に現れる順のスレッド一覧
    pub threads: Vec<SubjectThreadEntry>,
}

#[derive(Serialize, JsonSchema)]
pub struct SubjectThreadEntry {
    /// スレッドキー。先頭ゼロを保持するため文字列で返す
    pub thread_id: String,
    /// スレッドタイトル（NCR 復元後、trim しない）
    pub title: String,
    /// レス数
    pub res_count: u64,
}

fn is_zero(v: &usize) -> bool {
    *v == 0
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct FileInfoEntry {
    pub filename: String,
    pub thread_num: u32,
    pub thread_title: String,
    pub total_lines: usize,
    pub date_range: String,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct ListThreadsResponse {
    /// 作成日時降順。不明なものは末尾。同日時の場合はファイル名順。
    pub threads: Vec<LocalThreadEntry>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct LocalThreadEntry {
    pub file: String,
    pub title: String,
    pub post_count: usize,
    /// ファイル名中のスレッド ID から得られる Unix 時刻（秒）。不明なら null。
    pub created_at: Option<i64>,
}
