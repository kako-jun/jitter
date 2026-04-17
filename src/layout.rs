use crate::font::GlyphData;

/// Page-level metrics computed from glyphs and font size.
pub struct PageMetrics {
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub baseline_y: f64,
}

/// Compute canvas metrics from glyph advance widths and font size.
///
/// Guards against `units_per_em == 0` (broken fonts) by clamping to 1.
/// Baseline at ~73% of the height (1.1 / 1.5).
pub fn compute_metrics(glyphs: &[GlyphData], font_size: u32, units_per_em: u16) -> PageMetrics {
    let upem = if units_per_em == 0 { 1 } else { units_per_em };
    let scale = font_size as f64 / upem as f64;
    let total_advance: f64 = glyphs.iter().map(|g| g.advance_width as f64).sum();
    let width = (total_advance * scale).ceil() as u32;
    // Use 1.5x font size for height to accommodate ascenders/descenders.
    let height = (font_size as f64 * 1.5).ceil() as u32;
    // Baseline at ~73% of the height (1.1 / 1.5).
    let baseline_y = font_size as f64 * 1.1;
    PageMetrics {
        width,
        height,
        scale,
        baseline_y,
    }
}

/// Transform a font-space point (Y-up) to canvas-space (Y-down).
pub fn transform_point(x: f32, y: f32, scale: f64, offset_x: f64, baseline_y: f64) -> (f64, f64) {
    let sx = x as f64 * scale + offset_x;
    let sy = -(y as f64) * scale + baseline_y;
    (sx, sy)
}
