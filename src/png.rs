use crate::font::{GlyphData, PathCommand};
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
    let scale = font_size as f64 / units_per_em as f64;

    // Calculate total width and height (identical to svg::render_svg).
    let total_advance: f64 = glyphs.iter().map(|g| g.advance_width as f64).sum();
    let raw_width = (total_advance * scale).ceil() as u32;
    let raw_height = (font_size as f64 * 1.5).ceil() as u32;
    // Ensure Pixmap::new requirements (non-zero).
    let width = raw_width.max(1);
    let height = raw_height.max(1);
    let baseline_y = font_size as f64 * 1.1;

    let mut pixmap = Pixmap::new(width, height)
        .ok_or_else(|| format!("Invalid pixmap size: {width}x{height}"))?;

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

        let offset_x = cursor_x * scale;
        let mut pb = PathBuilder::new();
        let mut has_move = false;

        for cmd in commands {
            match *cmd {
                PathCommand::MoveTo(x, y) => {
                    let sx = (x as f64 * scale + offset_x) as f32;
                    let sy = (-(y as f64) * scale + baseline_y) as f32;
                    pb.move_to(sx, sy);
                    has_move = true;
                }
                PathCommand::LineTo(x, y) => {
                    let sx = (x as f64 * scale + offset_x) as f32;
                    let sy = (-(y as f64) * scale + baseline_y) as f32;
                    pb.line_to(sx, sy);
                }
                PathCommand::QuadTo(cx, cy, x, y) => {
                    let scx = (cx as f64 * scale + offset_x) as f32;
                    let scy = (-(cy as f64) * scale + baseline_y) as f32;
                    let sx = (x as f64 * scale + offset_x) as f32;
                    let sy = (-(y as f64) * scale + baseline_y) as f32;
                    pb.quad_to(scx, scy, sx, sy);
                }
                PathCommand::CurveTo(cx0, cy0, cx1, cy1, x, y) => {
                    let scx0 = (cx0 as f64 * scale + offset_x) as f32;
                    let scy0 = (-(cy0 as f64) * scale + baseline_y) as f32;
                    let scx1 = (cx1 as f64 * scale + offset_x) as f32;
                    let scy1 = (-(cy1 as f64) * scale + baseline_y) as f32;
                    let sx = (x as f64 * scale + offset_x) as f32;
                    let sy = (-(y as f64) * scale + baseline_y) as f32;
                    pb.cubic_to(scx0, scy0, scx1, scy1, sx, sy);
                }
                PathCommand::Close => {
                    pb.close();
                }
            }
        }

        if has_move {
            if let Some(path) = pb.finish() {
                pixmap.fill_path(
                    &path,
                    &paint,
                    FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }
        }

        cursor_x += glyph.advance_width as f64;
    }

    pixmap.encode_png().map_err(|e| e.to_string())
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
}
