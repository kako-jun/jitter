# jitter

Add natural handwriting-like variation to digital text.

## Why

Digital fonts produce identical glyphs every time the same character appears. Real handwriting never does — each letter has subtle differences in size, angle, position, and weight. jitter brings that organic variation back to digital text.

## Modes

### render

Takes text and a font file, applies per-character random transformations (rotation, scale, offset), and outputs an SVG or PNG (auto-detected from the `--output` extension).

```
jitter render "Hello, world!" --font my-font.ttf --output hello.svg
jitter render "Hello, world!" --font my-font.ttf --output hello.png
jitter render "手書き風" --font noto-sans-jp.otf --output tegaki.svg --intensity 0.8 --size 64
jitter render "reproducible" --font my-font.ttf --seed 42
```

### bake

Takes a TTF font and generates N variant glyphs per character, embedding them as an OpenType `rand` feature (alternate substitution). Text rendered with the output font in renderers that honor `rand` (HarfBuzz, CoreText, DirectWrite) will show naturally varied glyphs.

```
jitter bake my-font.ttf --alternates 4 --intensity 0.6
jitter bake my-font.ttf --output my-font-jittered.ttf
```

**Bake mode details**:
- Input: TTF or OTF/CFF
- Output: Always TTF (glyf format)
- OTF/CFF inputs are converted to quadratic outlines to fit the TTF glyf table

## Installation

```
cargo install jitter
```

## Usage

```
jitter render <TEXT> --font <FONT> [--output <FILE>] [--intensity <0.0-1.0>] [--size <PX>] [--seed <N>]
jitter bake <INPUT> [--output <FILE>] [--alternates <N>] [--intensity <0.0-1.0>]
```

### Options

- `--font`, `-f`: Path to a .ttf or .otf font file
- `--output`, `-o`: Output file path (default: `output.svg`). Format is auto-detected from the extension (`.svg` or `.png`, render mode only).
- `--intensity`, `-i`: How much variation to apply, from 0.0 (none) to 1.0 (maximum)
- `--size`, `-s`: Font size in pixels (render mode only, default: 48)
- `--seed`: Random seed (u64) for reproducible output. When omitted, output is non-deterministic (render mode only)
- `--alternates`, `-a`: Number of variant glyphs per character (bake mode only, default: 3)

## Output formats

- **SVG**: Vector output, scalable, editable.
- **PNG**: Rasterized output with transparent background (8-bit RGBA). Chosen automatically when the `--output` path ends in `.png`.

## Roadmap

1. ~~render mode (text + font -> SVG with per-character variation)~~ ✓
2. ~~bake mode phase A (TTF + `rand` feature)~~ ✓
3. ~~bake mode phase B (calt-based fallback for wider renderer support)~~ ✓
4. ~~bake mode phase C (OTF/CFF support)~~ ✓
5. my-font-craft integration (use jitter as a post-processing step)

## License

MIT
