use crate::font::{GlyphData, PathCommand};
use crate::layout;
use std::fmt::Write;

/// Generate an SVG string from jittered glyph data.
///
/// Converts from font coordinate space (Y-up) to SVG coordinate space (Y-down).
pub fn render_svg(
    glyphs: &[GlyphData],
    jittered_commands: &[Vec<PathCommand>],
    font_size: u32,
    units_per_em: u16,
) -> String {
    let m = layout::compute_metrics(glyphs, font_size, units_per_em);

    let mut svg = String::new();
    writeln!(
        &mut svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}">"#,
        w = m.width,
        h = m.height,
    )
    .unwrap();

    let mut cursor_x: f64 = 0.0;

    for (i, (glyph, commands)) in glyphs.iter().zip(jittered_commands.iter()).enumerate() {
        if commands.is_empty() {
            // Space or glyph with no outline — just advance
            cursor_x += glyph.advance_width as f64;
            continue;
        }

        let path_d = commands_to_path_d(commands, m.scale, cursor_x * m.scale, m.baseline_y);
        if !path_d.is_empty() {
            writeln!(&mut svg, r#"  <path id="g{i}" d="{path_d}" fill="black"/>"#,).unwrap();
        }

        cursor_x += glyph.advance_width as f64;
    }

    writeln!(&mut svg, "</svg>").unwrap();
    svg
}

/// Convert path commands to an SVG path `d` attribute string.
///
/// Applies coordinate transformation: font (Y-up) -> SVG (Y-down).
fn commands_to_path_d(
    commands: &[PathCommand],
    scale: f64,
    offset_x: f64,
    baseline_y: f64,
) -> String {
    let mut d = String::new();

    for cmd in commands {
        match *cmd {
            PathCommand::MoveTo(x, y) => {
                let (sx, sy) = layout::transform_point(x, y, scale, offset_x, baseline_y);
                write!(&mut d, "M{sx:.2} {sy:.2} ").unwrap();
            }
            PathCommand::LineTo(x, y) => {
                let (sx, sy) = layout::transform_point(x, y, scale, offset_x, baseline_y);
                write!(&mut d, "L{sx:.2} {sy:.2} ").unwrap();
            }
            PathCommand::QuadTo(cx, cy, x, y) => {
                let (scx, scy) = layout::transform_point(cx, cy, scale, offset_x, baseline_y);
                let (sx, sy) = layout::transform_point(x, y, scale, offset_x, baseline_y);
                write!(&mut d, "Q{scx:.2} {scy:.2} {sx:.2} {sy:.2} ").unwrap();
            }
            PathCommand::CurveTo(cx0, cy0, cx1, cy1, x, y) => {
                let (scx0, scy0) = layout::transform_point(cx0, cy0, scale, offset_x, baseline_y);
                let (scx1, scy1) = layout::transform_point(cx1, cy1, scale, offset_x, baseline_y);
                let (sx, sy) = layout::transform_point(x, y, scale, offset_x, baseline_y);
                write!(
                    &mut d,
                    "C{scx0:.2} {scy0:.2} {scx1:.2} {scy1:.2} {sx:.2} {sy:.2} "
                )
                .unwrap();
            }
            PathCommand::Close => {
                write!(&mut d, "Z ").unwrap();
            }
        }
    }

    d.trim_end().to_string()
}
