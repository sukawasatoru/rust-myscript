# Instructions for AI Agents

This guideline provides AI agents working on this codebase.

このコードベースは個人的に日常的に使用するユーティリティー群です。

## Do and Don'ts

- コードを変更したら reformat と lint と test を実行する
- OpenTelemetry は `BatchLogProcessor` を使用（SimpleLogProcessor はブロッキング問題あり）

## Build, Test, and Development Commands

- build: `cargo build`
- unit test: `cargo test --features test-helpers`
- lint: `cargo clippy`
- reformat: `cargo fmt`

## Testing Conventions

- `Fallible<()>` ではなく `unwrap()` を使用する

## Junie (AI Agent)

- Junie が使えるツール・機能は積極的に使用する
- bash tool を使用する前に Junie が使えるツール・機能で代替できないか検討し、bash tool によるパイプやリダイレクトの方が効果的な時に bash tool を使用する
- `cat` `head` の代わりに `open` / `open_entries_file` tool を使用できないか検討する
- `find` / `grep` の代わりに `search_project` tool を使用できないか検討する
- `ls` の代わりに `get_file_structure` tool を使用できないか検討する
- `sed` の代わりに `search_replace` / `multi_edit` / `rename_element` tool を使用できないか検討する
- `answer` / `submit` / `ask_user` tool を経由しないテキスト応答はユーザーに表示されないため、出力するべき情報は `answer` / `submit` / `ask_user` tool に記載する
