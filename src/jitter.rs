use crate::font::PathCommand;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// Upper bound on per-control-point offset, expressed as a fraction of
/// `units_per_em` (multiplied by `intensity`). Keeps internal shake subtle
/// relative to the rigid glyph translation (which uses 0.03).
const POINT_JITTER_SCALE: f64 = 0.01;
/// Upper bound on contour offset used for subtle stroke-weight variation,
/// expressed as a fraction of `units_per_em` (multiplied by `intensity`).
/// This sits below the rigid translation magnitude so glyphs feel "inked"
/// differently rather than resized outright.
const STROKE_WEIGHT_SCALE: f64 = 0.012;

/// Maximum shear magnitude at `intensity = 1.0`, expressed as the tangent of
/// the slant angle. `SHEAR_MAX = tan(5°) ≈ 0.0875`, i.e. the glyph leans up to
/// ~5° off vertical/horizontal along either axis.
const SHEAR_MAX: f64 = 0.087_488_663_525_924_01; // tan(5°)

type Point = (f64, f64);
type Normal = (f64, f64);

/// Per-glyph random transformation parameters.
struct GlyphTransform {
    /// Rotation angle in radians
    angle: f64,
    /// X offset in font units
    dx: f64,
    /// Y offset in font units
    dy: f64,
    /// Scale factor along X (independent of Y for anisotropic scaling)
    scale_x: f64,
    /// Scale factor along Y (independent of X for anisotropic scaling)
    scale_y: f64,
    /// Shear along X (tan of slant angle): `x' += shear_x * y`
    shear_x: f64,
    /// Shear along Y (tan of slant angle): `y' += shear_y * x`
    shear_y: f64,
    /// Glyph center X (for rotation/scale/shear pivot)
    cx: f64,
    /// Glyph center Y (for rotation/scale/shear pivot)
    cy: f64,
}

/// Apply jitter transformations to a list of path command sets (one per glyph).
///
/// Each glyph gets a random rotation, anisotropic scale, shear, and position
/// offset, followed by independent per-point noise on every bezier control
/// point.
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
                scale_x: 1.0 + rng.gen_range(-0.10..0.10) * intensity,
                scale_y: 1.0 + rng.gen_range(-0.10..0.10) * intensity,
                shear_x: rng.gen_range(-1.0..1.0) * SHEAR_MAX * intensity,
                shear_y: rng.gen_range(-1.0..1.0) * SHEAR_MAX * intensity,
                cx,
                cy,
            };
            let transformed = apply_transform(commands, &transform);
            let weighted =
                apply_stroke_weight_variation(&transformed, rng, intensity, units_per_em);
            apply_point_jitter(&weighted, rng, intensity, units_per_em)
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

/// Apply a contour-aware offset that makes the glyph look slightly thicker or
/// thinner without simply scaling the whole shape. A single random delta is
/// chosen per glyph; each contour then moves along its outward normal. For
/// counters/holes (opposite winding), the same delta naturally moves points in
/// the opposite geometric direction, which keeps "thicker" glyphs thickening
/// both the outer shell and the inner cutouts together.
fn apply_stroke_weight_variation<R: Rng>(
    commands: &[PathCommand],
    rng: &mut R,
    intensity: f64,
    units_per_em: f64,
) -> Vec<PathCommand> {
    if intensity == 0.0 || commands.is_empty() {
        return commands.to_vec();
    }
    let delta = rng.gen_range(-1.0..1.0) * units_per_em * STROKE_WEIGHT_SCALE * intensity;
    apply_stroke_weight_delta(commands, delta)
}

fn apply_stroke_weight_delta(commands: &[PathCommand], raw_delta: f64) -> Vec<PathCommand> {
    if raw_delta == 0.0 || commands.is_empty() {
        return commands.to_vec();
    }

    let contours = split_contours(commands);
    let contour_geometries: Vec<ContourGeometry> = contours
        .iter()
        .map(|contour| ContourGeometry::new(contour))
        .collect();

    contours
        .into_iter()
        .zip(contour_geometries.iter())
        .enumerate()
        .flat_map(|(index, (contour, geometry))| {
            let is_hole = contour_geometries
                .iter()
                .enumerate()
                .filter(|(other_index, other)| {
                    *other_index != index
                        && point_in_polygon(geometry.interior_probe, &other.on_curve_points)
                })
                .count()
                % 2
                == 1;
            offset_contour(&contour, geometry, raw_delta, is_hole)
        })
        .collect()
}

fn split_contours(commands: &[PathCommand]) -> Vec<Vec<PathCommand>> {
    let mut contours = Vec::new();
    let mut current = Vec::new();

    for command in commands {
        if matches!(command, PathCommand::MoveTo(..)) && !current.is_empty() {
            contours.push(current);
            current = Vec::new();
        }
        current.push(command.clone());
    }

    if !current.is_empty() {
        contours.push(current);
    }

    contours
}

struct ContourGeometry {
    on_curve_points: Vec<Point>,
    area: f64,
    interior_probe: Point,
}

impl ContourGeometry {
    fn new(contour: &[PathCommand]) -> Self {
        let on_curve_points = on_curve_points(contour);
        let area = signed_area(&on_curve_points);
        let interior_probe = interior_probe(&on_curve_points, area);
        Self {
            on_curve_points,
            area,
            interior_probe,
        }
    }
}

fn offset_contour(
    contour: &[PathCommand],
    geometry: &ContourGeometry,
    raw_delta: f64,
    is_hole: bool,
) -> Vec<PathCommand> {
    if geometry.on_curve_points.len() < 2 {
        return contour.to_vec();
    }

    let (min_x, min_y, max_x, max_y) = point_bounds(&geometry.on_curve_points);
    let max_delta = ((max_x - min_x).min(max_y - min_y) * 0.12).max(0.0);
    let delta = raw_delta.clamp(-max_delta, max_delta);
    if delta == 0.0 {
        return contour.to_vec();
    }

    let centroid = ((min_x + max_x) * 0.5, (min_y + max_y) * 0.5);
    let outward_sign = if geometry.area >= 0.0 { -1.0 } else { 1.0 };
    let fill_sign = if is_hole { -1.0 } else { 1.0 };
    let (adjusted_on_curve, normals) = offset_on_curve_points(
        &geometry.on_curve_points,
        centroid,
        outward_sign * fill_sign * delta,
    );
    rebuild_contour(
        contour,
        &adjusted_on_curve,
        &normals,
        outward_sign * fill_sign * delta,
    )
}

fn on_curve_points(contour: &[PathCommand]) -> Vec<Point> {
    let mut points = Vec::new();
    for command in contour {
        match command {
            PathCommand::MoveTo(x, y)
            | PathCommand::LineTo(x, y)
            | PathCommand::QuadTo(_, _, x, y)
            | PathCommand::CurveTo(_, _, _, _, x, y) => {
                points.push((*x as f64, *y as f64));
            }
            PathCommand::Close => {}
        }
    }
    points
}

fn point_bounds(points: &[Point]) -> (f64, f64, f64, f64) {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    for &(x, y) in points {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }

    (min_x, min_y, max_x, max_y)
}

fn signed_area(points: &[Point]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }

    let mut twice_area = 0.0;
    for i in 0..points.len() {
        let (x0, y0) = points[i];
        let (x1, y1) = points[(i + 1) % points.len()];
        twice_area += x0 * y1 - x1 * y0;
    }
    twice_area * 0.5
}

fn offset_on_curve_points(
    points: &[Point],
    centroid: Point,
    delta: f64,
) -> (Vec<Point>, Vec<Normal>) {
    let normals: Vec<Normal> = (0..points.len())
        .map(|index| {
            let prev = points[(index + points.len() - 1) % points.len()];
            let curr = points[index];
            let next = points[(index + 1) % points.len()];
            averaged_outward_normal(prev, curr, next, centroid)
        })
        .collect();
    let adjusted = points
        .iter()
        .zip(normals.iter())
        .map(|(&(x, y), &(nx, ny))| (x + nx * delta, y + ny * delta))
        .collect();
    (adjusted, normals)
}

fn averaged_outward_normal(prev: Point, curr: Point, next: Point, centroid: Point) -> Normal {
    let mut nx = 0.0;
    let mut ny = 0.0;

    if let Some((x, y)) = edge_left_normal(curr.0 - prev.0, curr.1 - prev.1) {
        nx += x;
        ny += y;
    }
    if let Some((x, y)) = edge_left_normal(next.0 - curr.0, next.1 - curr.1) {
        nx += x;
        ny += y;
    }

    normalize((nx, ny))
        .or_else(|| edge_left_normal(next.0 - prev.0, next.1 - prev.1))
        .or_else(|| normalize((curr.0 - centroid.0, curr.1 - centroid.1)))
        .unwrap_or((0.0, 0.0))
}

fn edge_left_normal(dx: f64, dy: f64) -> Option<Normal> {
    normalize((-dy, dx))
}

fn normalize((x, y): Normal) -> Option<Normal> {
    let len = (x * x + y * y).sqrt();
    if len <= f64::EPSILON {
        None
    } else {
        Some((x / len, y / len))
    }
}

fn interior_probe(points: &[Point], area: f64) -> Point {
    if points.is_empty() {
        return (0.0, 0.0);
    }
    if points.len() == 1 {
        return points[0];
    }

    let (min_x, min_y, max_x, max_y) = point_bounds(points);
    let epsilon = ((max_x - min_x).max(max_y - min_y) * 1e-3).max(1e-3);

    for index in 0..points.len() {
        let curr = points[index];
        let next = points[(index + 1) % points.len()];
        let dx = next.0 - curr.0;
        let dy = next.1 - curr.1;
        if let Some((left_x, left_y)) = edge_left_normal(dx, dy) {
            let interior_sign = if area >= 0.0 { 1.0 } else { -1.0 };
            let midpoint = ((curr.0 + next.0) * 0.5, (curr.1 + next.1) * 0.5);
            return (
                midpoint.0 + left_x * interior_sign * epsilon,
                midpoint.1 + left_y * interior_sign * epsilon,
            );
        }
    }

    points[0]
}

fn point_in_polygon(point: Point, polygon: &[Point]) -> bool {
    if polygon.len() < 3 {
        return false;
    }

    let mut inside = false;
    let mut prev = polygon[polygon.len() - 1];
    for &curr in polygon {
        let intersects = ((curr.1 > point.1) != (prev.1 > point.1))
            && (point.0 < (prev.0 - curr.0) * (point.1 - curr.1) / (prev.1 - curr.1) + curr.0);
        if intersects {
            inside = !inside;
        }
        prev = curr;
    }
    inside
}

fn rebuild_contour(
    contour: &[PathCommand],
    adjusted_on_curve: &[Point],
    normals: &[Normal],
    delta: f64,
) -> Vec<PathCommand> {
    let mut on_curve_index = 0;
    contour
        .iter()
        .map(|command| match command {
            PathCommand::MoveTo(..) => {
                let point = adjusted_on_curve[on_curve_index];
                PathCommand::MoveTo(point.0 as f32, point.1 as f32)
            }
            PathCommand::LineTo(..) => {
                on_curve_index += 1;
                let point = adjusted_on_curve[on_curve_index];
                PathCommand::LineTo(point.0 as f32, point.1 as f32)
            }
            PathCommand::QuadTo(cx, cy, ..) => {
                let start_index = on_curve_index;
                let end_index = (on_curve_index + 1) % adjusted_on_curve.len();
                let (offset_x, offset_y) =
                    segment_control_offset(normals[start_index], normals[end_index], delta);
                on_curve_index = end_index;
                let end = adjusted_on_curve[end_index];
                PathCommand::QuadTo(
                    *cx + offset_x as f32,
                    *cy + offset_y as f32,
                    end.0 as f32,
                    end.1 as f32,
                )
            }
            PathCommand::CurveTo(cx0, cy0, cx1, cy1, ..) => {
                let start_index = on_curve_index;
                let end_index = (on_curve_index + 1) % adjusted_on_curve.len();
                let (offset_x, offset_y) =
                    segment_control_offset(normals[start_index], normals[end_index], delta);
                on_curve_index = end_index;
                let end = adjusted_on_curve[end_index];
                PathCommand::CurveTo(
                    *cx0 + offset_x as f32,
                    *cy0 + offset_y as f32,
                    *cx1 + offset_x as f32,
                    *cy1 + offset_y as f32,
                    end.0 as f32,
                    end.1 as f32,
                )
            }
            PathCommand::Close => PathCommand::Close,
        })
        .collect()
}

fn segment_control_offset(start_normal: Normal, end_normal: Normal, delta: f64) -> Point {
    let average = normalize((start_normal.0 + end_normal.0, start_normal.1 + end_normal.1))
        .unwrap_or(start_normal);
    (average.0 * delta, average.1 * delta)
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

/// Apply anisotropic scale + shear, rotation (around glyph center), and
/// translation to a point.
fn transform_point(x: f64, y: f64, t: &GlyphTransform) -> (f64, f64) {
    // Translate to glyph center
    let dx = x - t.cx;
    let dy = y - t.cy;
    // 2x2 linear transform: anisotropic scale on the diagonal, shear on the
    // off-diagonal. Applied before rotation so rotation sees the stretched
    // glyph (matches the "slightly tall, slightly slanted" intent).
    let sx = t.scale_x * dx + t.shear_x * dy;
    let sy = t.shear_y * dx + t.scale_y * dy;
    // Rotate
    let cos = t.angle.cos();
    let sin = t.angle.sin();
    let rx = sx * cos - sy * sin;
    let ry = sx * sin + sy * cos;
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

    /// Directly verify that `apply_transform` honors anisotropic scale: with
    /// `scale_x = 2.0` and everything else identity, the point `(1, 1)` maps
    /// to `(2, 1)`. This locks down the x/y-independent scaling semantics.
    #[test]
    fn apply_transform_applies_anisotropic_scale() {
        let t = GlyphTransform {
            angle: 0.0,
            dx: 0.0,
            dy: 0.0,
            scale_x: 2.0,
            scale_y: 1.0,
            shear_x: 0.0,
            shear_y: 0.0,
            cx: 0.0,
            cy: 0.0,
        };
        let input = vec![PathCommand::MoveTo(1.0, 1.0)];
        let out = apply_transform(&input, &t);
        match out[0] {
            PathCommand::MoveTo(x, y) => {
                assert!((x - 2.0).abs() < 1e-5, "x should be 2.0, got {x}");
                assert!((y - 1.0).abs() < 1e-5, "y should be 1.0, got {y}");
            }
            _ => panic!("expected MoveTo"),
        }
    }

    /// Directly verify that `apply_transform` honors shear: with `shear_x =
    /// 0.5` and everything else identity, `(0, 1)` maps to `(0.5, 1)` because
    /// `shear_x * y = 0.5` is added to the x coordinate.
    #[test]
    fn apply_transform_applies_shear() {
        let t = GlyphTransform {
            angle: 0.0,
            dx: 0.0,
            dy: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            shear_x: 0.5,
            shear_y: 0.0,
            cx: 0.0,
            cy: 0.0,
        };
        let input = vec![PathCommand::MoveTo(0.0, 1.0)];
        let out = apply_transform(&input, &t);
        match out[0] {
            PathCommand::MoveTo(x, y) => {
                assert!((x - 0.5).abs() < 1e-5, "x should be 0.5, got {x}");
                assert!((y - 1.0).abs() < 1e-5, "y should be 1.0, got {y}");
            }
            _ => panic!("expected MoveTo"),
        }
    }

    #[test]
    fn stroke_weight_delta_expands_outer_and_shrinks_inner_contours() {
        let input = vec![
            PathCommand::MoveTo(0.0, 0.0),
            PathCommand::LineTo(100.0, 0.0),
            PathCommand::LineTo(100.0, 100.0),
            PathCommand::LineTo(0.0, 100.0),
            PathCommand::Close,
            PathCommand::MoveTo(30.0, 30.0),
            PathCommand::LineTo(30.0, 70.0),
            PathCommand::LineTo(70.0, 70.0),
            PathCommand::LineTo(70.0, 30.0),
            PathCommand::Close,
        ];

        let output = apply_stroke_weight_delta(&input, 10.0);
        let contours = split_contours(&output);
        assert_eq!(contours.len(), 2);

        let outer = on_curve_points(&contours[0]);
        let inner = on_curve_points(&contours[1]);
        let (outer_min_x, outer_min_y, outer_max_x, outer_max_y) = point_bounds(&outer);
        let (inner_min_x, inner_min_y, inner_max_x, inner_max_y) = point_bounds(&inner);

        assert!(outer_min_x < 0.0);
        assert!(outer_min_y < 0.0);
        assert!(outer_max_x > 100.0);
        assert!(outer_max_y > 100.0);
        assert!(inner_min_x > 30.0);
        assert!(inner_min_y > 30.0);
        assert!(inner_max_x < 70.0);
        assert!(inner_max_y < 70.0);
    }

    #[test]
    fn stroke_weight_delta_expands_multiple_outer_islands_even_with_mixed_winding() {
        let input = vec![
            PathCommand::MoveTo(0.0, 0.0),
            PathCommand::LineTo(100.0, 0.0),
            PathCommand::LineTo(100.0, 100.0),
            PathCommand::LineTo(0.0, 100.0),
            PathCommand::Close,
            PathCommand::MoveTo(240.0, 0.0),
            PathCommand::LineTo(240.0, 100.0),
            PathCommand::LineTo(140.0, 100.0),
            PathCommand::LineTo(140.0, 0.0),
            PathCommand::Close,
        ];

        let output = apply_stroke_weight_delta(&input, 10.0);
        let contours = split_contours(&output);
        assert_eq!(contours.len(), 2);

        let first = on_curve_points(&contours[0]);
        let second = on_curve_points(&contours[1]);
        let (first_min_x, _, first_max_x, _) = point_bounds(&first);
        let (second_min_x, _, second_max_x, _) = point_bounds(&second);

        assert!(first_min_x < 0.0);
        assert!(first_max_x > 100.0);
        assert!(second_min_x < 140.0);
        assert!(second_max_x > 240.0);
    }

    #[test]
    fn stroke_weight_delta_handles_quadratic_contours() {
        let input = vec![
            PathCommand::MoveTo(50.0, 0.0),
            PathCommand::QuadTo(100.0, 0.0, 100.0, 50.0),
            PathCommand::QuadTo(100.0, 100.0, 50.0, 100.0),
            PathCommand::QuadTo(0.0, 100.0, 0.0, 50.0),
            PathCommand::QuadTo(0.0, 0.0, 50.0, 0.0),
            PathCommand::Close,
        ];

        let output = apply_stroke_weight_delta(&input, 8.0);
        let contour = split_contours(&output).pop().unwrap();
        let points = on_curve_points(&contour);
        let (min_x, min_y, max_x, max_y) = point_bounds(&points);

        assert!(min_x < 0.0);
        assert!(min_y < 0.0);
        assert!(max_x > 100.0);
        assert!(max_y > 100.0);
    }

    #[test]
    fn stroke_weight_delta_handles_cubic_contours() {
        let input = vec![
            PathCommand::MoveTo(50.0, 0.0),
            PathCommand::CurveTo(85.0, 0.0, 100.0, 15.0, 100.0, 50.0),
            PathCommand::CurveTo(100.0, 85.0, 85.0, 100.0, 50.0, 100.0),
            PathCommand::CurveTo(15.0, 100.0, 0.0, 85.0, 0.0, 50.0),
            PathCommand::CurveTo(0.0, 15.0, 15.0, 0.0, 50.0, 0.0),
            PathCommand::Close,
        ];

        let output = apply_stroke_weight_delta(&input, 8.0);
        let contour = split_contours(&output).pop().unwrap();
        let points = on_curve_points(&contour);
        let (min_x, min_y, max_x, max_y) = point_bounds(&points);

        assert!(min_x < 0.0);
        assert!(min_y < 0.0);
        assert!(max_x > 100.0);
        assert!(max_y > 100.0);
    }
}
