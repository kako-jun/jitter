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

- `src/main.rs`: CLIエントリーポイント、サブコマンド定義（clap）
- `src/font.rs`: フォント読み込み、グリフアウトライン抽出（skrifa）
- `src/jitter.rs`: 変形エンジン（回転・位置ずれ・拡縮をグリフ単位で適用）
- `src/svg.rs`: SVG出力（フォント座標→SVG座標変換）
- render モード: テキスト+フォント → SVG
- bake モード: TTF/OTF フォント → calt付き TTF

## 技術ルール

- コミットメッセージに Co-Authored-By を付けない
