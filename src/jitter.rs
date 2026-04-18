use crate::font::PathCommand;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// Upper bound on per-control-point offset, expressed as a fraction of
/// `units_per_em` (multiplied by `intensity`). Keeps internal shake subtle
/// relative to the rigid glyph translation (which uses 0.03).
const POINT_JITTER_SCALE: f64 = 0.01;

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
/// produces identical output. When `None`, a non-deterministic RNG is used.
pub fn apply_jitter(
    glyph_commands: &[Vec<PathCommand>],
    intensity: f64,
    units_per_em: f64,
    seed: Option<u64>,
) -> Vec<Vec<PathCommand>> {
    match seed {
        Some(s) => {
            let mut rng = ChaCha8Rng::seed_from_u64(s);
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
            let transformed = apply_transform(commands, &transform);
            apply_point_jitter(&transformed, rng, intensity, units_per_em)
        })
        .collect()
}

/// Apply jitter to a single glyph's path commands.
///
/// Convenience wrapper for `apply_jitter` when you only have one glyph and
/// want the transformed commands back directly instead of as a singleton
/// vector.
pub fn apply_jitter_one(
    commands: &[PathCommand],
    intensity: f64,
    units_per_em: f64,
) -> Vec<PathCommand> {
    // Route the thread_rng path through the same per-glyph pipeline as
    // `apply_jitter` to keep the two implementations in lockstep. The
    // per-call vec allocation is fine — this is not a hot path.
    let mut rng = rand::thread_rng();
    run_with_rng(&mut rng, &[commands.to_vec()], intensity, units_per_em)
        .into_iter()
        .next()
        .unwrap_or_default()
}

/// Apply independent small offsets to every on-curve / off-curve control
/// point in the path. This sits *after* the rigid glyph transform so each
/// point shakes on top of the per-glyph rotation/scale/translation.
///
/// `Close` commands are passed through unchanged. Offsets are uniform in
/// `±units_per_em * POINT_JITTER_SCALE * intensity` per axis (half-open:
/// upper bound exclusive, matching `Rng::gen_range(-1.0..1.0)`) and are
/// drawn independently for each coordinate of each point.
fn apply_point_jitter<R: Rng>(
    commands: &[PathCommand],
    rng: &mut R,
    intensity: f64,
    units_per_em: f64,
) -> Vec<PathCommand> {
    let amp = units_per_em * POINT_JITTER_SCALE * intensity;
    // Draw a single-axis offset in font units. `amp` stays in f64 for the
    // multiplication, then the final point coordinate is f32 so we cast
    // once at the return site instead of at every call site.
    let offset = |rng: &mut R| -> f32 {
        if amp == 0.0 {
            0.0
        } else {
            (rng.gen_range(-1.0..1.0) * amp) as f32
        }
    };
    commands
        .iter()
        .map(|cmd| match cmd {
            PathCommand::MoveTo(x, y) => PathCommand::MoveTo(*x + offset(rng), *y + offset(rng)),
            PathCommand::LineTo(x, y) => PathCommand::LineTo(*x + offset(rng), *y + offset(rng)),
            PathCommand::QuadTo(cx, cy, x, y) => PathCommand::QuadTo(
                *cx + offset(rng),
                *cy + offset(rng),
                *x + offset(rng),
                *y + offset(rng),
            ),
            PathCommand::CurveTo(cx0, cy0, cx1, cy1, x, y) => PathCommand::CurveTo(
                *cx0 + offset(rng),
                *cy0 + offset(rng),
                *cx1 + offset(rng),
                *cy1 + offset(rng),
                *x + offset(rng),
                *y + offset(rng),
            ),
            PathCommand::Close => PathCommand::Close,
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

    #[test]
    fn same_seed_produces_identical_output() {
        let input = sample_input();
        let a = apply_jitter(&input, 0.7, 1000.0, Some(42));
        let b = apply_jitter(&input, 0.7, 1000.0, Some(42));
        assert_eq!(a, b, "same seed must produce identical output");
    }

    #[test]
    fn different_seeds_produce_different_output() {
        let input = sample_input();
        let a = apply_jitter(&input, 0.7, 1000.0, Some(1));
        let b = apply_jitter(&input, 0.7, 1000.0, Some(2));
        assert_ne!(a, b, "different seeds must produce different output");
    }

    #[test]
    fn no_seed_uses_thread_rng_and_still_varies() {
        let input = sample_input();
        let out = apply_jitter(&input, 0.7, 1000.0, None);
        assert_eq!(out.len(), input.len());
        // intensity > 0 なので入力と異なるはず
        assert_ne!(out, input);
    }

    #[test]
    fn intensity_zero_is_identity_regardless_of_seed() {
        let input = sample_input();
        let a = apply_jitter(&input, 0.0, 1000.0, Some(1));
        let b = apply_jitter(&input, 0.0, 1000.0, Some(9999));
        assert_eq!(a, b);
        // Per-point jitter must also be a strict identity at intensity=0,
        // otherwise float-noise offsets could silently leak into every point.
        assert_eq!(a, input);
    }

    /// With intensity > 0, per-point jitter should break the rigid-body
    /// invariant: i.e. the transformed points cannot all be explained by a
    /// single affine transform applied to the input.
    ///
    /// Strategy: use a single glyph with several collinear points on y=0.
    /// A pure rigid transform (rotation + uniform scale + translation)
    /// preserves collinearity. After per-point jitter, at least one point
    /// should deviate from the line through the first two output points.
    #[test]
    fn per_point_jitter_varies_internal_shape() {
        let input = vec![vec![
            PathCommand::MoveTo(0.0, 0.0),
            PathCommand::LineTo(100.0, 0.0),
            PathCommand::LineTo(200.0, 0.0),
            PathCommand::LineTo(300.0, 0.0),
            PathCommand::LineTo(400.0, 0.0),
            PathCommand::Close,
        ]];

        let out = apply_jitter(&input, 1.0, 1000.0, Some(42));
        assert_eq!(out.len(), 1);
        let glyph = &out[0];

        // Collect endpoints of MoveTo/LineTo in order.
        let points: Vec<(f64, f64)> = glyph
            .iter()
            .filter_map(|cmd| match cmd {
                PathCommand::MoveTo(x, y) | PathCommand::LineTo(x, y) => {
                    Some((*x as f64, *y as f64))
                }
                _ => None,
            })
            .collect();
        assert_eq!(points.len(), 5);

        // Line through the first two output points.
        let (x0, y0) = points[0];
        let (x1, y1) = points[1];
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len = (dx * dx + dy * dy).sqrt();
        assert!(len > 0.0, "first two points must not coincide");

        // Signed perpendicular distance from the line for each remaining point.
        // If this were a pure rigid transform of collinear input, all
        // distances would be 0 (within float noise). Per-point jitter must
        // push at least one point off the line by a meaningful amount.
        let mut max_off_line: f64 = 0.0;
        for &(x, y) in &points[2..] {
            let off = ((x - x0) * dy - (y - y0) * dx).abs() / len;
            max_off_line = max_off_line.max(off);
        }
        assert!(
            max_off_line > 0.5,
            "per-point jitter should break collinearity (off-line = {max_off_line})"
        );
    }
}
