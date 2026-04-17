use skrifa::instance::Size;
use skrifa::raw::FileRef;
use skrifa::raw::TableProvider;
use skrifa::MetadataProvider;
use std::path::Path;

/// A single path command extracted from a glyph outline.
#[derive(Debug, Clone, PartialEq)]
pub enum PathCommand {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    QuadTo(f32, f32, f32, f32),
    CurveTo(f32, f32, f32, f32, f32, f32),
    Close,
}

/// A glyph with its outline path commands and advance width.
#[derive(Debug, Clone)]
pub struct GlyphData {
    pub commands: Vec<PathCommand>,
    pub advance_width: f32,
}

/// Pen that collects path commands from skrifa's outline API.
struct CollectPen {
    commands: Vec<PathCommand>,
}

impl CollectPen {
    fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }
}

impl skrifa::outline::OutlinePen for CollectPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.commands.push(PathCommand::MoveTo(x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.commands.push(PathCommand::LineTo(x, y));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.commands.push(PathCommand::QuadTo(cx0, cy0, x, y));
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.commands
            .push(PathCommand::CurveTo(cx0, cy0, cx1, cy1, x, y));
    }

    fn close(&mut self) {
        self.commands.push(PathCommand::Close);
    }
}

/// Load a font file and extract glyph data for each character in the text.
///
/// Returns a vector of `GlyphData` (one per character) and the units per em value.
pub fn load_glyphs(font_path: &Path, text: &str) -> Result<(Vec<GlyphData>, u16), String> {
    let font_data =
        std::fs::read(font_path).map_err(|e| format!("Failed to read font file: {e}"))?;

    let font_ref = match FileRef::new(&font_data) {
        Ok(FileRef::Font(f)) => f,
        Ok(FileRef::Collection(c)) => c
            .get(0)
            .map_err(|e| format!("Failed to get font from collection: {e}"))?,
        Err(e) => return Err(format!("Failed to parse font: {e}")),
    };

    let units_per_em = font_ref.head().map_err(|e| format!("{e}"))?.units_per_em();
    let charmap = font_ref.charmap();
    let outlines = font_ref.outline_glyphs();
    let location = font_ref.axes().location::<&[(&str, f32)]>(&[]);
    let glyph_metrics = font_ref.glyph_metrics(Size::unscaled(), &location);

    let mut glyphs = Vec::new();

    for ch in text.chars() {
        let glyph_id = charmap
            .map(ch)
            .ok_or_else(|| format!("Character '{ch}' not found in font"))?;

        let advance_width = glyph_metrics.advance_width(glyph_id).unwrap_or(0.0);

        let outline_glyph = outlines.get(glyph_id);

        let mut pen = CollectPen::new();
        if let Some(glyph) = outline_glyph {
            glyph
                .draw(Size::unscaled(), &mut pen)
                .map_err(|e| format!("Failed to draw glyph for '{ch}': {e}"))?;
        }

        glyphs.push(GlyphData {
            commands: pen.commands,
            advance_width,
        });
    }

    Ok((glyphs, units_per_em))
}
