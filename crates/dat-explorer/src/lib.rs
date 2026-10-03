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

//! dat ファイルの取得・解析、MCP サーバーと TUI ビュアー。
//!
//! # 構成
//! - [`feature`] — MCP tool・応答制限、および viewer の画面・操作・非同期結果管理。
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
//! viewer は [`application::backend::Backend`] enum を経由し、Direct/MCP を切り替える。
//! MCP サーバーは Backend を経由せず ThreadService を利用する。
//! viewer の設定は ProjectDirs の config_dir()/viewer.toml、閲覧状態は
//! state_dir()（未対応 OS では data_local_dir()）/viewer-state.toml に保存する。
//! ProjectDirs の識別子は ("com", "sukawasatoru", "dat-explorer")。
//! 一覧は既存の `{prefix}_{num}_{id}.dat` を対象とし、id の Unix 時刻降順。
//! 読書位置はレス番号と表示テキスト内の byte offset で保存し、幅変更時に再配置する。
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
