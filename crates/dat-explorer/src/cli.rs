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

use clap::{Parser, Subcommand, ValueHint};
use std::path::PathBuf;

/// 5ch の dat ファイルを取得・読み取り・検索するツール。
#[derive(Debug, Parser)]
#[command(verbatim_doc_comment)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// MCP サーバーを起動する。
    ///
    /// 提供する tool:
    ///   read_posts     保存済み dat のレスを読み取る。
    ///   list_threads   保存済み dat のスレッド一覧を表示する。
    ///   search_posts   保存済み dat をキーワード・投稿者 ID で検索する。
    ///   fetch_dat      スレッドを取得し、UTF-8 の dat として保存する。
    ///   fetch_subject  板のスレッド一覧を取得する。
    #[command(verbatim_doc_comment)]
    Mcp {
        /// dat ファイルの読み込み先ディレクトリ。
        ///
        /// fetch_dat の相対保存パスも、このディレクトリを基準に解決する。
        #[arg(value_hint = ValueHint::DirPath)]
        dat_dir: PathBuf,

        /// 読み取り・検索結果の安全上限（50,000 文字）を無効化する。
        ///
        /// LM Studio の応答サイズ制約に合わせた上限を解除する。
        /// tool 引数 max_body_chars に明示した文字数制限は引き続き適用される。
        /// max_body_chars が 0 の場合は文字数を制限しない。
        #[arg(long)]
        disable_body_limit: bool,
    },
    /// 保存済み dat を TUI で閲覧する。
    ///
    /// Enter: スレッドを開く。↑/↓・PageUp/PageDown・ホイール: 移動。
    /// Ctrl+N/P: ↓/↑と同じ移動。本文の URL を左クリックするとブラウザで開く。
    /// 存在するレスへの >>N にマウスを重ねるとプレビュー。枠内のホイールで移動。
    /// 枠内のアンカーもホバーで辿れる。Esc: 最前面のプレビューを閉じる。
    /// 枠内の URL も左クリックで開く。
    /// F2: マウス操作を切り替える。OFF の間は端末側で文字選択・コピーが可能。
    /// r: 再読み込み。Esc: 一覧へ戻る／終了。F1・Ctrl+,: 取得経路の設定。
    /// 読書位置と取得経路は次回起動時にも復元する。
    #[command(verbatim_doc_comment)]
    Viewer {
        /// dat ファイルの読み込み先ディレクトリ。
        #[arg(value_hint = ValueHint::DirPath)]
        dat_dir: PathBuf,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition() {
        Cli::command().debug_assert();
    }

    #[test]
    fn mcp_arguments() {
        let cli =
            Cli::try_parse_from(["dat-explorer", "mcp", "./dat", "--disable-body-limit"]).unwrap();
        let Command::Mcp {
            dat_dir,
            disable_body_limit,
        } = cli.command
        else {
            panic!("expected mcp");
        };
        assert_eq!(dat_dir, PathBuf::from("./dat"));
        assert!(disable_body_limit);

        let cli = Cli::try_parse_from(["dat-explorer", "mcp", "./dat"]).unwrap();
        let Command::Mcp {
            disable_body_limit, ..
        } = cli.command
        else {
            panic!("expected mcp");
        };
        assert!(!disable_body_limit);
    }

    #[test]
    fn viewer_arguments() {
        let cli = Cli::try_parse_from(["dat-explorer", "viewer", "./dat"]).unwrap();
        assert!(
            matches!(cli.command, Command::Viewer { dat_dir } if dat_dir == std::path::Path::new("./dat"))
        );
        assert!(Cli::try_parse_from(["dat-explorer", "viewer"]).is_err());
    }

    #[test]
    fn requires_subcommand_and_directory() {
        assert!(Cli::try_parse_from(["dat-explorer"]).is_err());
        assert!(Cli::try_parse_from(["dat-explorer", "./dat"]).is_err());
        assert!(Cli::try_parse_from(["dat-explorer", "mcp"]).is_err());
    }
}
