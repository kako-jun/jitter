use crate::font::PathCommand;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

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
    /// Glyph center X (for rotation/scale pivot)
    cx: f64,
    /// Glyph center Y (for rotation/scale pivot)
    cy: f64,
}

/// Apply jitter transformations to a list of path command sets (one per glyph).
///
/// Each glyph gets a random rotation, position offset, and scale variation.
/// The `intensity` parameter (0.0-1.0) controls the magnitude of the variation.
/// `units_per_em` is used to scale the position offset relative to glyph size.
/// When `seed` is `Some`, a deterministic RNG is used so that the same input
/// produces identical output. When `None`, the thread-local RNG is used.
pub fn apply_jitter(
    glyph_commands: &[Vec<PathCommand>],
    intensity: f64,
    units_per_em: f64,
    seed: Option<u64>,
) -> Vec<Vec<PathCommand>> {
    match seed {
        Some(s) => {
            let mut rng = StdRng::seed_from_u64(s);
            run_with_rng(&mut rng, glyph_commands, intensity, units_per_em)
        }
        None => {
            let mut rng = rand::thread_rng();
            run_with_rng(&mut rng, glyph_commands, intensity, units_per_em)
        }
    }
}

fn run_with_rng<R: Rng>(
    rng: &mut R,
    glyph_commands: &[Vec<PathCommand>],
    intensity: f64,
    units_per_em: f64,
) -> Vec<Vec<PathCommand>> {
    glyph_commands
        .iter()
        .map(|commands| {
            let (cx, cy) = compute_center(commands);
            let transform = GlyphTransform {
                angle: rng.gen_range(-5.0..5.0_f64).to_radians() * intensity,
                dx: rng.gen_range(-1.0..1.0) * units_per_em * 0.03 * intensity,
                dy: rng.gen_range(-1.0..1.0) * units_per_em * 0.03 * intensity,
                scale: 1.0 + rng.gen_range(-0.05..0.05) * intensity,
                cx,
                cy,
            };
            apply_transform(commands, &transform)
        })
        .collect()
}

/// Compute the bounding box center of a set of path commands.
fn compute_center(commands: &[PathCommand]) -> (f64, f64) {
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;

    for cmd in commands {
        let points: Vec<(f64, f64)> = match cmd {
            PathCommand::MoveTo(x, y) | PathCommand::LineTo(x, y) => {
                vec![(*x as f64, *y as f64)]
            }
            PathCommand::QuadTo(cx, cy, x, y) => {
                vec![(*cx as f64, *cy as f64), (*x as f64, *y as f64)]
            }
            PathCommand::CurveTo(cx0, cy0, cx1, cy1, x, y) => {
                vec![
                    (*cx0 as f64, *cy0 as f64),
                    (*cx1 as f64, *cy1 as f64),
                    (*x as f64, *y as f64),
                ]
            }
            PathCommand::Close => vec![],
        };

        for (x, y) in points {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }

    if min_x == f64::MAX {
        (0.0, 0.0)
    } else {
        ((min_x + max_x) / 2.0, (min_y + max_y) / 2.0)
    }
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

/// Apply scale, rotation (around glyph center), and translation to a point.
fn transform_point(x: f64, y: f64, t: &GlyphTransform) -> (f64, f64) {
    // Translate to glyph center
    let dx = x - t.cx;
    let dy = y - t.cy;
    // Scale
    let dx = dx * t.scale;
    let dy = dy * t.scale;
    // Rotate
    let cos = t.angle.cos();
    let sin = t.angle.sin();
    let rx = dx * cos - dy * sin;
    let ry = dx * sin + dy * cos;
    // Translate back and apply position offset
    (rx + t.cx + t.dx, ry + t.cy + t.dy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_input() -> Vec<Vec<PathCommand>> {
        vec![
            vec![
                PathCommand::MoveTo(0.0, 0.0),
                PathCommand::LineTo(100.0, 100.0),
                PathCommand::QuadTo(50.0, 150.0, 100.0, 200.0),
                PathCommand::CurveTo(10.0, 20.0, 30.0, 40.0, 50.0, 60.0),
                PathCommand::Close,
            ],
            vec![
                PathCommand::MoveTo(200.0, 200.0),
                PathCommand::LineTo(300.0, 250.0),
                PathCommand::Close,
            ],
        ]
    }

    fn debug_repr(v: &[Vec<PathCommand>]) -> String {
        format!("{v:?}")
    }

    #[test]
    fn same_seed_produces_identical_output() {
        let input = sample_input();
        let a = apply_jitter(&input, 0.7, 1000.0, Some(42));
        let b = apply_jitter(&input, 0.7, 1000.0, Some(42));
        assert_eq!(
            debug_repr(&a),
            debug_repr(&b),
            "same seed must produce identical output"
        );
    }

    #[test]
    fn different_seeds_produce_different_output() {
        let input = sample_input();
        let a = apply_jitter(&input, 0.7, 1000.0, Some(1));
        let b = apply_jitter(&input, 0.7, 1000.0, Some(2));
        assert_ne!(
            debug_repr(&a),
            debug_repr(&b),
            "different seeds must produce different output"
        );
    }
}
