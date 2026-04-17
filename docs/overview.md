# jitter — Design Overview

## Problem

Digital fonts are perfectly consistent: every "a" is identical to every other "a". This is a feature for readability, but it strips away the organic quality of handwriting. When you want text that feels human — for games, art, posters, or personal projects — you need variation.

Existing approaches either require manual work (drawing multiple glyphs by hand) or are locked inside design tools (Photoshop/Illustrator effects). There is no simple CLI tool that takes a font and adds controlled randomness.

## Solution

jitter applies per-character random transformations to text, producing output that looks like it was written by a human hand rather than a machine. It operates in two modes:

### render mode

Input: text string + font file (.ttf/.otf)
Output: SVG or PNG image

Each character gets independent random variation in:
- **Rotation**: slight tilt (e.g. -3 to +3 degrees)
- **Scale**: minor size changes (e.g. 0.95x to 1.05x)
- **Position offset**: vertical and horizontal drift
- **Stroke weight**: subtle thickness variation (future)

The `intensity` parameter (0.0 to 1.0) controls how much variation is applied. At 0.0, output is identical to the original font. At 1.0, maximum variation is applied.

An optional `--seed <u64>` parameter makes output reproducible: the same text, font, intensity, and seed always produce identical SVG. Omitting `--seed` uses a non-deterministic RNG (previous behavior).

### bake mode

Input: font file (.ttf/.otf)
Output: modified font file with OpenType `calt` alternates

Instead of rendering text to an image, bake mode modifies the font itself. For each glyph, it generates N alternate versions with baked-in transformations and adds OpenType `calt` (contextual alternates) rules that cycle through them. Any application that renders text with the output font automatically gets natural variation — no special tooling required.

## Differentiation

| Tool | Approach | Output | Automation |
|------|----------|--------|------------|
| Photoshop/Illustrator | Manual per-character adjustment | Raster/vector | None |
| Calligraphr | Scan handwriting samples | Font file | Semi-auto |
| FontForge scripting | Python API for font editing | Font file | Scriptable but complex |
| **jitter** | Algorithmic variation from any font | Image or font | Fully automated CLI |

jitter's key advantage is that it works with any existing font and requires zero manual input beyond choosing parameters.

## Architecture

```
CLI (clap)
├── render: text + font -> SVG
│   ├── font.rs — Font loading & glyph outline extraction (skrifa)
│   ├── jitter.rs — Per-character random transforms (rotation, scale, offset)
│   └── svg.rs — SVG output (font coords → SVG coords, path generation)
└── bake: font -> font (not yet implemented)
    ├── Font loading (skrifa)
    ├── Glyph duplication with transforms
    ├── calt feature table generation
    └── Font serialization (write-fonts)
```

## Future integration

jitter is designed to complement [my-font-craft](https://github.com/kako-jun/my-font-craft), a tool for creating fonts from handwritten samples. The workflow:

1. Create a base font from your handwriting with my-font-craft
2. Run `jitter bake` to add natural variation
3. Use the output font anywhere — every character automatically varies
