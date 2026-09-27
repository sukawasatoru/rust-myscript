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

//! dat ファイルの取得・解析と MCP サーバー。
//!
//! # 構成
//! - [`feature`] — MCP tool と応答の加工・文字数制限。
//! - [`application`] — 読み込み・検索・取得保存の共通手順。
//! - [`data`] — ファイル IO、取得先別クライアント、内部 mapper による形式変換。
//! - [`model`] — 共通の値型、範囲指定、検索条件、本文中の URL の扱い。
//! - [`dto`] — MCP の要求・応答型。
//!
//! 主な依存方向は feature → application → data → model。
//! 上位層は model を参照できるが、data 内の mapper は公開しない。
//! application は解析済みの [`model::LoadedThread`] を使う。
//! 保存には [`model::DownloadedDat`] を渡し、表示用レスから dat を再構築しない。
//! MCP 固有の回答文字数制限は feature に閉じ込める。
//!
//! # 開発
//! リポジトリルートで `cargo fmt`、`cargo clippy`、
//! `cargo test --features test-helpers` を実行する。
//! この crate のみのテストは `cargo test -p dat-explorer`。
//! HTTP テストはローカルサーバー、stdio テストは実バイナリを使用する。

pub mod application;
pub mod data;
pub mod dto;
pub mod feature;
pub mod model;
