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
use std::process::ExitCode;
use tracing::Level;

/// セッションをまたいでメモを保存・参照できる MCP サーバー。
///
/// 覚えておきたい情報をキー付きのメモとして保存し、別のセッションから参照できます。
/// MCP クライアントから、メモの保存・取得・部分編集・削除と、キーの一覧表示ができます。
/// 同じメモを使い続けるには、同じ保存先を指定してください。
///
/// 保存先:
/// メモは指定したディレクトリに保存します。新しい保存先は自動で初期化します。
/// 旧形式のデータがある場合は、利用前に移行が必要です。
/// 移行手順は mcp-memo <data_dir> migrate --help を参照してください。
///
/// 変更履歴:
/// メモの作成・更新・削除は自動で Git 履歴に記録します。
/// 履歴は git -C <data_dir> log などで確認できます。
/// MCP 経由での履歴の取得・復元には対応していません。
///
/// 利用上の注意:
/// 履歴の保存に失敗した場合、エラーでもメモ自体は変更されていることがあります。
/// 再試行する前に get_memo で現在の内容を確認してください。
/// 実行中は、外部エディターや Git コマンドで保存先を変更しないでください。
#[derive(Debug, Parser)]
#[command(verbatim_doc_comment)]
struct Opt {
    /// メモと変更履歴を保存するディレクトリ。
    #[clap(value_hint = ValueHint::DirPath)]
    data_dir: PathBuf,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 旧形式のメモとバックアップを、新しい履歴管理方式へ移行します。
    ///
    /// 旧サーバーをすべて停止し、保存先全体をバックアップしてから実行してください。
    /// まず --dry-run で移行対象を確認し、問題がなければ移行を実行します。
    ///
    /// 実行手順:
    ///   mcp-memo <data_dir> migrate --dry-run
    ///   mcp-memo <data_dir> migrate
    ///
    /// 旧バックアップは、ファイル名の日時を日本時間（UTC+09:00）として履歴に取り込みます。
    /// 現在のメモは移行時点の日時で記録します。
    /// 現在のメモは変更・削除せず、削除済みのメモも復活させません。
    /// 移行成功後、取り込み済みの旧バックアップだけを削除します。
    /// 対象外のファイルは残し、ディレクトリは空になった場合だけ削除します。
    ///
    /// 保存先直下と backup/<key>/ 内の .txt ファイルを対象にします。
    /// 移行対象が読み込めない、または形式が不正な場合は、移行を中止します。
    /// 削除に失敗しても履歴への移行は完了しています。原因を解消して再実行してください。
    /// 移行済みの場合、履歴は追加せず、取り込み時と内容が一致する旧バックアップだけを削除します。
    #[command(verbatim_doc_comment)]
    Migrate {
        /// 入力を検証して件数を表示し、Git 履歴の作成や旧バックアップの削除は行いません。
        ///
        /// 排他用の .mcp-memo.lock は作成します。
        #[arg(long)]
        dry_run: bool,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(Level::INFO)
        .init();

    let opt = Opt::parse();
    let result =
        match opt.command {
            None => mcp_memo::run_mcp_server(opt.data_dir).await,
            Some(Command::Migrate { dry_run }) => mcp_memo::migrate(opt.data_dir, dry_run)
                .await
                .map(|report| {
                    if report.already_migrated {
                        eprintln!(
                            "Migration already completed; {} imported backups {}.",
                            report.backup_count,
                            if report.dry_run {
                                "would be removed"
                            } else {
                                "removed"
                            }
                        );
                    } else {
                        eprintln!(
                            "{}: {} backups, {} current memos (legacy timezone: Asia/Tokyo).",
                            if report.dry_run {
                                "Dry run"
                            } else {
                                "Migration completed"
                            },
                            report.backup_count,
                            report.memo_count
                        );
                    }
                }),
        };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn long_help_includes_storage_documentation() {
        let error = Opt::try_parse_from(["mcp-memo", "--help"]).unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
        let help = error.to_string();
        for text in [
            "セッションをまたいでメモを保存・参照できる MCP サーバー。",
            "同じ保存先を指定してください",
            "mcp-memo <data_dir> migrate --help",
            "保存先:",
            "変更履歴:",
            "git -C <data_dir> log",
            "利用上の注意:",
            "エラーでもメモ自体は変更されていることがあります",
            "get_memo で現在の内容を確認してください",
            "メモと変更履歴を保存するディレクトリ。",
        ] {
            assert!(help.contains(text), "missing help text: {text}\n{help}");
        }
        assert!(!help.contains("ディスク故障"));
        assert!(!help.contains(".git/mcp-memo-format"));
    }

    #[test]
    fn migration_help_includes_procedure_and_cautions() {
        let error = Opt::try_parse_from(["mcp-memo", "/data", "migrate", "--help"]).unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
        let help = error.to_string();
        for text in [
            "旧サーバーをすべて停止",
            "保存先全体をバックアップしてから実行してください",
            "  mcp-memo <data_dir> migrate --dry-run\n  mcp-memo <data_dir> migrate",
            "UTC+09:00",
            "現在のメモは移行時点の日時で記録します",
            "現在のメモは変更・削除せず",
            "取り込み済みの旧バックアップだけを削除します",
            "対象外のファイルは残し",
            "ディレクトリは空になった場合だけ削除します",
            "復活させません",
            "移行対象が読み込めない、または形式が不正な場合は、移行を中止します",
            "Git 履歴の作成や旧バックアップの削除は行いません",
            ".mcp-memo.lock",
            "原因を解消して再実行してください",
            "移行済みの場合、履歴は追加せず",
        ] {
            assert!(help.contains(text), "missing help text: {text}\n{help}");
        }
    }

    #[test]
    fn opt_should_parse_valid_args() {
        Opt::command().debug_assert();
        assert!(
            Opt::try_parse_from(["mcp-memo", "/data"])
                .unwrap()
                .command
                .is_none()
        );
        assert!(matches!(
            Opt::try_parse_from(["mcp-memo", "/data", "migrate", "--dry-run"])
                .unwrap()
                .command,
            Some(Command::Migrate { dry_run: true })
        ));
    }
}
