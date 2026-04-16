# jitter

テキスト+フォントを入力し、1文字ごとにランダムな「癖」変形を加えて出力するCLIツール。

## ビルド・テスト

```bash
cargo build
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

## アーキテクチャ

- `src/main.rs`: CLIエントリーポイント、サブコマンド定義
- render モード: テキスト+フォント → SVG/PNG
- bake モード: フォント → calt付きフォント

## 技術ルール

- コミットメッセージに Co-Authored-By を付けない
