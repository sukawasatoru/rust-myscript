# Instructions for AI Agents

This guideline provides AI agents working on this codebase.

このコードベースは個人的に日常的に使用するユーティリティー群です。

## Do and Don'ts

- Do: コードを変更したら reformat と lint と test を実行する
- Do: OpenTelemetry は `BatchLogProcessor` を使用（SimpleLogProcessor はブロッキング問題あり）

## Build, Test, and Development Commands

- build: `cargo build`
- unit test: `cargo test --features test-helpers`
- lint: `cargo clippy`
- reformat: `cargo fmt`
