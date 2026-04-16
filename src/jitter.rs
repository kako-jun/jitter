use crate::font::PathCommand;
use rand::Rng;

/// Per-glyph random transformation parameters.
struct GlyphTransform {
    /// Rotation angle in radians
    angle: f64,
    /// X offset in font units
    dx: f64,
    /// Y offset in font units
    dy: f64,
    /// Scale factor
    scale: f64,
}

/// Apply jitter transformations to a list of path command sets (one per glyph).
///
/// Each glyph gets a random rotation, position offset, and scale variation.
/// The `intensity` parameter (0.0-1.0) controls the magnitude of the variation.
/// `font_size` is used to scale the position offset.
pub fn apply_jitter(
    glyph_commands: &[Vec<PathCommand>],
    intensity: f64,
    font_size: f64,
) -> Vec<Vec<PathCommand>> {
    let mut rng = rand::thread_rng();

    glyph_commands
        .iter()
        .map(|commands| {
            let transform = GlyphTransform {
                angle: rng.gen_range(-5.0..5.0_f64).to_radians() * intensity,
                dx: rng.gen_range(-1.0..1.0) * font_size * 0.05 * intensity,
                dy: rng.gen_range(-1.0..1.0) * font_size * 0.05 * intensity,
                scale: 1.0 + rng.gen_range(-0.05..0.05) * intensity,
            };
            apply_transform(commands, &transform)
        })
        .collect()
}

fn apply_transform(commands: &[PathCommand], t: &GlyphTransform) -> Vec<PathCommand> {
    commands
        .iter()
        .map(|cmd| match cmd {
            PathCommand::MoveTo(x, y) => {
                let (nx, ny) = transform_point(*x as f64, *y as f64, t);
                PathCommand::MoveTo(nx as f32, ny as f32)
            }
            PathCommand::LineTo(x, y) => {
                let (nx, ny) = transform_point(*x as f64, *y as f64, t);
                PathCommand::LineTo(nx as f32, ny as f32)
            }
            PathCommand::QuadTo(cx, cy, x, y) => {
                let (ncx, ncy) = transform_point(*cx as f64, *cy as f64, t);
                let (nx, ny) = transform_point(*x as f64, *y as f64, t);
                PathCommand::QuadTo(ncx as f32, ncy as f32, nx as f32, ny as f32)
            }
            PathCommand::CurveTo(cx0, cy0, cx1, cy1, x, y) => {
                let (ncx0, ncy0) = transform_point(*cx0 as f64, *cy0 as f64, t);
                let (ncx1, ncy1) = transform_point(*cx1 as f64, *cy1 as f64, t);
                let (nx, ny) = transform_point(*x as f64, *y as f64, t);
                PathCommand::CurveTo(
                    ncx0 as f32,
                    ncy0 as f32,
                    ncx1 as f32,
                    ncy1 as f32,
                    nx as f32,
                    ny as f32,
                )
            }
            PathCommand::Close => PathCommand::Close,
        })
        .collect()
}

/// Apply scale, rotation, and translation to a point.
fn transform_point(x: f64, y: f64, t: &GlyphTransform) -> (f64, f64) {
    // Scale
    let x = x * t.scale;
    let y = y * t.scale;
    // Rotate
    let cos = t.angle.cos();
    let sin = t.angle.sin();
    let rx = x * cos - y * sin;
    let ry = x * sin + y * cos;
    // Translate
    (rx + t.dx, ry + t.dy)
}
