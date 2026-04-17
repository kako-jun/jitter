use crate::font::{GlyphData, PathCommand};
use crate::layout;
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Transform};

/// Render jittered glyph data to a transparent PNG byte buffer.
///
/// Uses the same coordinate transform as `svg::render_svg`:
/// font coordinate space (Y-up) -> pixel coordinate space (Y-down).
pub fn render_png(
    glyphs: &[GlyphData],
    jittered_commands: &[Vec<PathCommand>],
    font_size: u32,
    units_per_em: u16,
) -> Result<Vec<u8>, String> {
    let pixmap = build_pixmap(glyphs, jittered_commands, font_size, units_per_em)?;
    pixmap.encode_png().map_err(|e| e.to_string())
}

/// Build a tiny-skia Pixmap by rasterizing the jittered glyph data.
///
/// Exposed at crate level so tests can inspect the raw pixel buffer.
pub(crate) fn build_pixmap(
    glyphs: &[GlyphData],
    jittered_commands: &[Vec<PathCommand>],
    font_size: u32,
    units_per_em: u16,
) -> Result<Pixmap, String> {
    let m = layout::compute_metrics(glyphs, font_size, units_per_em);

    // Ensure Pixmap::new requirements (non-zero).
    let width = m.width.max(1);
    let height = m.height.max(1);

    let mut pixmap =
        Pixmap::new(width, height).ok_or_else(|| format!("Pixmap too large: {width}x{height}"))?;

    let mut paint = Paint::default();
    paint.set_color(Color::BLACK);
    paint.anti_alias = true;

    let mut cursor_x: f64 = 0.0;

    for (glyph, commands) in glyphs.iter().zip(jittered_commands.iter()) {
        if commands.is_empty() {
            // Space or glyph with no outline — just advance.
            cursor_x += glyph.advance_width as f64;
            continue;
        }

        let offset_x = cursor_x * m.scale;
        let mut pb = PathBuilder::new();

        for cmd in commands {
            match *cmd {
                PathCommand::MoveTo(x, y) => {
                    let (sx, sy) = layout::transform_point(x, y, m.scale, offset_x, m.baseline_y);
                    pb.move_to(sx as f32, sy as f32);
                }
                PathCommand::LineTo(x, y) => {
                    let (sx, sy) = layout::transform_point(x, y, m.scale, offset_x, m.baseline_y);
                    pb.line_to(sx as f32, sy as f32);
                }
                PathCommand::QuadTo(cx, cy, x, y) => {
                    let (scx, scy) =
                        layout::transform_point(cx, cy, m.scale, offset_x, m.baseline_y);
                    let (sx, sy) = layout::transform_point(x, y, m.scale, offset_x, m.baseline_y);
                    pb.quad_to(scx as f32, scy as f32, sx as f32, sy as f32);
                }
                PathCommand::CurveTo(cx0, cy0, cx1, cy1, x, y) => {
                    let (scx0, scy0) =
                        layout::transform_point(cx0, cy0, m.scale, offset_x, m.baseline_y);
                    let (scx1, scy1) =
                        layout::transform_point(cx1, cy1, m.scale, offset_x, m.baseline_y);
                    let (sx, sy) = layout::transform_point(x, y, m.scale, offset_x, m.baseline_y);
                    pb.cubic_to(
                        scx0 as f32,
                        scy0 as f32,
                        scx1 as f32,
                        scy1 as f32,
                        sx as f32,
                        sy as f32,
                    );
                }
                PathCommand::Close => {
                    pb.close();
                }
            }
        }

        if let Some(path) = pb.finish() {
            pixmap.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }

        cursor_x += glyph.advance_width as f64;
    }

    Ok(pixmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

    #[test]
    fn empty_input_does_not_panic() {
        let bytes = render_png(&[], &[], 48, 1000).expect("empty input should render");
        assert!(bytes.len() >= 8);
        assert_eq!(&bytes[0..8], &PNG_SIGNATURE);
    }

    #[test]
    fn png_signature() {
        let glyphs = vec![GlyphData {
            advance_width: 500.0,
            commands: vec![],
        }];
        let cmds = vec![vec![
            PathCommand::MoveTo(0.0, 0.0),
            PathCommand::LineTo(500.0, 0.0),
            PathCommand::LineTo(500.0, 500.0),
            PathCommand::LineTo(0.0, 500.0),
            PathCommand::Close,
        ]];
        let bytes = render_png(&glyphs, &cmds, 48, 1000).expect("dummy input should render");
        assert_eq!(&bytes[0..8], &PNG_SIGNATURE);
    }

    #[test]
    fn renders_opaque_pixels() {
        let glyphs = vec![GlyphData {
            advance_width: 500.0,
            commands: vec![],
        }];
        let cmds = vec![vec![
            PathCommand::MoveTo(0.0, 0.0),
            PathCommand::LineTo(500.0, 0.0),
            PathCommand::LineTo(500.0, 500.0),
            PathCommand::LineTo(0.0, 500.0),
            PathCommand::Close,
        ]];
        let pixmap = build_pixmap(&glyphs, &cmds, 48, 1000).expect("dummy input should render");
        let data = pixmap.data();
        let has_opaque = data.chunks(4).any(|px| px[3] != 0);
        assert!(has_opaque, "expected at least one non-transparent pixel");
    }
}
