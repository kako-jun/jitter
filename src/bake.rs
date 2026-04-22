//! Bake mode: generate a new TTF with alternate glyphs and a GSUB `rand` feature.
//!
//! Phase C scope:
//! - Input is TTF or OTF (CFF / CFF2). Output is always TTF (glyf/loca).
//! - Cubic Bézier outlines from CFF sources are approximated as quadratic
//!   curves so they can be stored in the TrueType `glyf` table.
//! - For each non-empty simple glyph (excluding `.notdef`), N alternates are
//!   created by re-running jitter, then registered in a minimal GSUB table
//!   using the `rand` feature (OpenType 1.8) with AlternateSubstFormat1.
//! - `.notdef` (gid 0) is preserved unchanged and excluded from alternates.
//! - Composite glyphs are consumed via skrifa's pen and re-emitted as flat
//!   simple glyphs (structure flattened, visual appearance preserved).
//! - Most tables (name, OS/2, cmap, ...) are copied from the input font
//!   verbatim. `post` is downgraded to format 3.0 because the glyph count
//!   changed and the original format 2 glyph-name index would be stale.
//!
//! Shapers that honour the `rand` feature will pick a random alternate each
//! time the glyph is laid out, producing the handwriting-like variation baked
//! into the font itself. calt / contextual alternates are out of scope here
//! and tracked as a follow-up.
//!
//! Compatibility note: `skrifa 0.26` uses `read-fonts 0.25`, while
//! `write-fonts 0.31` uses `read-fonts 0.24`. Both are in the dependency graph.
//! We use skrifa to read the font for outline extraction, and parse the same
//! bytes again through `write_fonts::read` to hand to `FontBuilder`.

use crate::font::PathCommand;
use crate::jitter;
use kurbo::BezPath;
use skrifa::instance::Size;
use skrifa::outline::OutlinePen;
use skrifa::raw::{FileRef, TableProvider as SkrifaTableProvider};
use skrifa::{GlyphId, MetadataProvider};
use std::path::Path;
use write_fonts::read::types::Tag;
use write_fonts::read::{FontRead, FontRef as WfFontRef};
use write_fonts::tables::glyf::{GlyfLocaBuilder, Glyph, SimpleGlyph};
use write_fonts::tables::gsub::{AlternateSet, AlternateSubstFormat1, Gsub, SubstitutionLookup};
use write_fonts::tables::head::Head;
use write_fonts::tables::hhea::Hhea;
use write_fonts::tables::hmtx::{Hmtx, LongMetric};
use write_fonts::tables::layout::{
    CoverageFormat1, CoverageTable, Feature, FeatureList, FeatureRecord, LangSys, Lookup,
    LookupFlag, LookupList, Script, ScriptList, ScriptRecord,
};
use write_fonts::tables::maxp::Maxp;
use write_fonts::tables::post::Post;
use write_fonts::types::Version16Dot16;
use write_fonts::FontBuilder;

/// Bake jitter variation into a font file.
///
/// Reads the TTF at `input_path`, generates `alternates` jittered variants per
/// glyph, and writes a new TTF with the `rand` GSUB feature to `output_path`.
pub fn bake_font(
    input_path: &Path,
    output_path: &Path,
    alternates: u32,
    intensity: f64,
) -> Result<(), String> {
    if alternates == 0 {
        return Err("alternates must be at least 1".to_string());
    }

    let font_data =
        std::fs::read(input_path).map_err(|e| format!("Failed to read font file: {e}"))?;

    // Parse with skrifa for outline extraction.
    let skrifa_font = match FileRef::new(&font_data) {
        Ok(FileRef::Font(f)) => f,
        Ok(FileRef::Collection(_)) => {
            return Err("Font collections (.ttc/.otc) are not yet supported".to_string())
        }
        Err(e) => return Err(format!("Failed to parse font: {e}")),
    };

    // Parse the same bytes through write-fonts' read-fonts to satisfy the
    // builder's type expectations (the two read-fonts versions are distinct
    // types in the dependency graph even though the wire format is the same).
    let wf_font =
        WfFontRef::new(&font_data).map_err(|e| format!("Failed to re-parse font: {e}"))?;

    let num_glyphs = skrifa_font
        .maxp()
        .map_err(|e| format!("Failed to read maxp: {e}"))?
        .num_glyphs();
    let units_per_em = skrifa_font
        .head()
        .map_err(|e| format!("Failed to read head: {e}"))?
        .units_per_em() as f64;

    let outlines = skrifa_font.outline_glyphs();

    // Extract each original glyph's outline.
    // `originals[i]` is the source for gid=i.
    let mut originals: Vec<OriginalGlyph> = Vec::with_capacity(num_glyphs as usize);
    for gid_u16 in 0..num_glyphs {
        let gid = GlyphId::new(gid_u16 as u32);

        let outline = outlines.get(gid);
        let commands = if let Some(glyph) = outline {
            let mut pen = CollectPen::new();
            glyph
                .draw(Size::unscaled(), &mut pen)
                .map_err(|e| format!("Failed to draw glyph {gid_u16}: {e}"))?;
            pen.commands
        } else {
            Vec::new()
        };

        originals.push(OriginalGlyph { commands });
    }

    // Compose the glyph list: originals first, then append alternates.
    // Track alternate gids per origin so we can build the GSUB Alternate
    // substitution afterwards.
    let mut alt_map: Vec<Vec<u16>> = vec![Vec::new(); num_glyphs as usize];
    let mut new_glyphs: Vec<Glyph> = Vec::with_capacity(num_glyphs as usize);
    let mut new_metrics: Vec<LongMetric> = Vec::with_capacity(num_glyphs as usize);

    // Read original hmtx so we can preserve exact side bearings for originals.
    // We need hhea's number_of_long_metrics to interpret hmtx.
    let hhea_bytes_ro = wf_font
        .table_data(Tag::new(b"hhea"))
        .ok_or_else(|| "Font is missing 'hhea' table".to_string())?;
    let hhea_ro = write_fonts::read::tables::hhea::Hhea::read(hhea_bytes_ro)
        .map_err(|e| format!("Failed to read hhea: {e}"))?;
    let num_long_metrics = hhea_ro.number_of_h_metrics() as usize;
    let hmtx_bytes_ro = wf_font
        .table_data(Tag::new(b"hmtx"))
        .ok_or_else(|| "Font is missing 'hmtx' table".to_string())?;
    let original_hmtx = write_fonts::read::tables::hmtx::Hmtx::read(
        hmtx_bytes_ro,
        num_long_metrics as u16,
    )
    .map_err(|e| format!("Failed to read hmtx: {e}"))?;

    // Originals pass: build Glyph + LongMetric for each gid.
    for (gid, orig) in originals.iter().enumerate() {
        let glyph = build_simple_glyph(&orig.commands, gid as u16)?;
        new_glyphs.push(glyph);

        let (advance, lsb) = resolve_original_hmtx(&original_hmtx, gid, num_long_metrics);
        new_metrics.push(LongMetric::new(advance, lsb));
    }

    // Alternates pass: only for non-empty glyphs, and never for .notdef (gid 0).
    let mut next_gid: u32 = num_glyphs as u32;
    for gid in 0..num_glyphs as usize {
        if gid == 0 {
            // .notdef is the fallback glyph; do not vary it.
            continue;
        }
        let orig = &originals[gid];
        if orig.commands.is_empty() {
            continue;
        }
        for _ in 0..alternates {
            // Guard against u16 overflow before allocating a new gid.
            if next_gid > u16::MAX as u32 {
                return Err(format!(
                    "Too many glyphs after baking: {} exceeds {}",
                    next_gid,
                    u16::MAX
                ));
            }

            // Run jitter once per alternate to get a fresh variation.
            let jittered = jitter::apply_jitter_one(&orig.commands, intensity, units_per_em);

            let glyph = build_simple_glyph(&jittered, next_gid as u16)?;
            new_glyphs.push(glyph);
            // Alternate inherits the advance width of its origin.
            let advance = new_metrics[gid].advance;
            let lsb = new_metrics[gid].side_bearing;
            new_metrics.push(LongMetric::new(advance, lsb));

            alt_map[gid].push(next_gid as u16);
            next_gid += 1;
        }
    }

    let total_glyphs = new_glyphs.len();
    // total_glyphs is always <= next_gid, which was bounded above, but keep a
    // defensive check so the cast is always sound.
    if total_glyphs > u16::MAX as usize {
        return Err(format!(
            "Too many glyphs after baking: {} exceeds {}",
            total_glyphs,
            u16::MAX
        ));
    }
    let total_glyphs_u16 = total_glyphs as u16;

    // Build glyf + loca.
    let mut glyf_builder = GlyfLocaBuilder::new();
    for (i, glyph) in new_glyphs.iter().enumerate() {
        glyf_builder
            .add_glyph(glyph)
            .map_err(|e| format!("Failed to add glyph {i}: {e}"))?;
    }
    let (glyf_table, loca_table, loca_format) = glyf_builder.build();

    // Build updated head, maxp, hhea based on originals. We parse the bytes
    // via write-fonts' FontRead so we get owned write-fonts tables.
    let head_bytes = wf_font
        .table_data(Tag::new(b"head"))
        .ok_or_else(|| "Font is missing 'head' table".to_string())?;
    let mut head = Head::read(head_bytes).map_err(|e| format!("Failed to own head: {e}"))?;
    head.index_to_loc_format = match loca_format {
        write_fonts::tables::loca::LocaFormat::Short => 0,
        write_fonts::tables::loca::LocaFormat::Long => 1,
    };

    let maxp_bytes = wf_font
        .table_data(Tag::new(b"maxp"))
        .ok_or_else(|| "Font is missing 'maxp' table".to_string())?;
    let mut maxp = Maxp::read(maxp_bytes).map_err(|e| format!("Failed to own maxp: {e}"))?;
    maxp.num_glyphs = total_glyphs_u16;

    let hhea_bytes = wf_font
        .table_data(Tag::new(b"hhea"))
        .ok_or_else(|| "Font is missing 'hhea' table".to_string())?;
    let mut hhea = Hhea::read(hhea_bytes).map_err(|e| format!("Failed to own hhea: {e}"))?;
    hhea.number_of_h_metrics = total_glyphs_u16;

    let hmtx = Hmtx::new(new_metrics, Vec::new());

    // Build a post table downgraded to format 3.0. The original post may have
    // been format 2.0 with a glyph_name_index sized for the pre-bake glyph
    // count, which would be inconsistent with the new maxp.num_glyphs. Format
    // 3.0 stores no glyph names, so it is always consistent.
    let post = build_post_v3(&wf_font)?;

    // Build a minimal GSUB: script DFLT / langsys dflt / feature rand / AlternateSubst lookup.
    let gsub = build_gsub(&alt_map)?;

    // Assemble the FontBuilder. We add the rebuilt tables first, then call
    // `copy_missing_tables` — which only inserts tables we haven't added — so
    // the originals (name, OS/2, cmap, ...) come through untouched while our
    // replacements (including the format-3 post) win.
    let mut builder = FontBuilder::new();
    builder
        .add_table(&head)
        .map_err(|e| format!("head: {e}"))?
        .add_table(&maxp)
        .map_err(|e| format!("maxp: {e}"))?
        .add_table(&hhea)
        .map_err(|e| format!("hhea: {e}"))?
        .add_table(&hmtx)
        .map_err(|e| format!("hmtx: {e}"))?
        .add_table(&glyf_table)
        .map_err(|e| format!("glyf: {e}"))?
        .add_table(&loca_table)
        .map_err(|e| format!("loca: {e}"))?
        .add_table(&post)
        .map_err(|e| format!("post: {e}"))?
        .add_table(&gsub)
        .map_err(|e| format!("GSUB: {e}"))?;
    builder.copy_missing_tables(wf_font);

    let out = builder.build();

    // Sanity: verify the output re-parses and has consistent counts.
    verify_baked_font(&out, total_glyphs_u16)?;

    std::fs::write(output_path, &out).map_err(|e| format!("Failed to write output: {e}"))?;

    Ok(())
}

/// Verify that a freshly-baked font is internally consistent.
///
/// Re-parses via skrifa and checks that `maxp.num_glyphs` and the hmtx
/// long-metric count match the value we wrote.
fn verify_baked_font(data: &[u8], expected_num_glyphs: u16) -> Result<(), String> {
    // Re-parse through write-fonts (cheap consistency check).
    if WfFontRef::new(data).is_err() {
        return Err("bake produced a font that failed write-fonts re-parse".to_string());
    }

    // Re-parse through skrifa too (goes through a different read-fonts copy
    // so this exercises both code paths).
    let file = skrifa::raw::FileRef::new(data)
        .map_err(|e| format!("bake produced a font that failed skrifa re-parse: {e}"))?;
    let font = match file {
        skrifa::raw::FileRef::Font(f) => f,
        skrifa::raw::FileRef::Collection(_) => {
            return Err("bake produced a collection, expected a single font".to_string())
        }
    };

    let num_glyphs = font
        .maxp()
        .map_err(|e| format!("bake produced font whose maxp is unreadable: {e}"))?
        .num_glyphs();
    if num_glyphs != expected_num_glyphs {
        return Err(format!(
            "bake produced inconsistent font: maxp.num_glyphs={num_glyphs}, expected {expected_num_glyphs}"
        ));
    }

    let hhea = font
        .hhea()
        .map_err(|e| format!("bake produced font whose hhea is unreadable: {e}"))?;
    let num_long_metrics = hhea.number_of_h_metrics();
    if num_long_metrics != expected_num_glyphs {
        return Err(format!(
            "bake produced inconsistent font: hhea.number_of_long_metrics={num_long_metrics}, expected {expected_num_glyphs}"
        ));
    }

    let hmtx = font
        .hmtx()
        .map_err(|e| format!("bake produced font whose hmtx is unreadable: {e}"))?;
    let actual_h_metrics = hmtx.h_metrics().len();
    if actual_h_metrics != expected_num_glyphs as usize {
        return Err(format!(
            "bake produced inconsistent font: hmtx.h_metrics.len={actual_h_metrics}, expected {expected_num_glyphs}"
        ));
    }

    Ok(())
}

/// Build a format 3.0 post table, reusing the header metadata (italic angle,
/// underline metrics, etc.) from the input font's post. Format 3 has no
/// glyph-name index, so it stays consistent after the glyph count changes.
fn build_post_v3(wf_font: &WfFontRef<'_>) -> Result<Post, String> {
    let post_bytes = wf_font
        .table_data(Tag::new(b"post"))
        .ok_or_else(|| "Font is missing 'post' table".to_string())?;
    let mut post = Post::read(post_bytes).map_err(|e| format!("Failed to own post: {e}"))?;
    post.version = Version16Dot16::VERSION_3_0;
    post.num_glyphs = None;
    post.glyph_name_index = None;
    post.string_data = None;
    Ok(post)
}

/// Per-glyph extraction result.
struct OriginalGlyph {
    commands: Vec<PathCommand>,
}

/// OutlinePen that collects path commands from skrifa's outline API.
/// Composite glyphs come through as drawn outlines via skrifa, so we don't
/// see a composite marker directly. Cubic curves (legal in CFF) are collected
/// as-is and converted to quadratic approximations later in `build_simple_glyph`.
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

impl OutlinePen for CollectPen {
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

/// Convert a jitter path command list into a `write-fonts` SimpleGlyph via
/// kurbo. Empty commands -> empty glyph (glyf allows zero-length entries).
/// Cubic Bézier segments are approximated as quadratic curves so the result
/// is always valid for the TrueType `glyf` table.
fn build_simple_glyph(commands: &[PathCommand], gid: u16) -> Result<Glyph, String> {
    if commands.is_empty() {
        return Ok(Glyph::Empty);
    }

    let mut path = BezPath::new();
    let mut has_cubic = false;
    for cmd in commands {
        match *cmd {
            PathCommand::MoveTo(x, y) => path.move_to((x as f64, y as f64)),
            PathCommand::LineTo(x, y) => path.line_to((x as f64, y as f64)),
            PathCommand::QuadTo(cx, cy, x, y) => {
                path.quad_to((cx as f64, cy as f64), (x as f64, y as f64))
            }
            PathCommand::CurveTo(cx0, cy0, cx1, cy1, x, y) => {
                has_cubic = true;
                path.curve_to(
                    (cx0 as f64, cy0 as f64),
                    (cx1 as f64, cy1 as f64),
                    (x as f64, y as f64),
                );
            }
            PathCommand::Close => path.close_path(),
        }
    }

    let final_path = if has_cubic {
        cubic_to_quadratic(&path, 0.5)
    } else {
        path
    };

    match SimpleGlyph::from_bezpath(&final_path) {
        Ok(g) => Ok(Glyph::Simple(g)),
        Err(_) => {
            eprintln!("warning: glyph {gid} produced an invalid outline and was emitted as empty");
            Ok(Glyph::Empty)
        }
    }
}

/// Approximate all cubic Bézier segments in a `BezPath` as quadratic curves.
///
/// Uses `kurbo::CubicBez::to_quads` with the given accuracy (in font units).
/// The output contains only `MoveTo`, `LineTo`, `QuadTo`, and `ClosePath`
/// elements, making it suitable for `SimpleGlyph::from_bezpath`.
fn cubic_to_quadratic(path: &BezPath, accuracy: f64) -> BezPath {
    let mut quad = BezPath::new();
    let mut current = kurbo::Point::ORIGIN;
    let mut start = kurbo::Point::ORIGIN;

    for el in path.elements() {
        match *el {
            kurbo::PathEl::MoveTo(p) => {
                quad.move_to(p);
                current = p;
                start = p;
            }
            kurbo::PathEl::LineTo(p) => {
                quad.line_to(p);
                current = p;
            }
            kurbo::PathEl::QuadTo(p1, p2) => {
                quad.quad_to(p1, p2);
                current = p2;
            }
            kurbo::PathEl::CurveTo(p1, p2, p3) => {
                let c = kurbo::CubicBez::new(current, p1, p2, p3);
                for (_, _, q) in c.to_quads(accuracy) {
                    quad.quad_to(q.p1, q.p2);
                }
                current = p3;
            }
            kurbo::PathEl::ClosePath => {
                quad.close_path();
                current = start;
            }
        }
    }
    quad
}

/// Look up `(advance, lsb)` for an original glyph from the input hmtx.
///
/// Glyphs at index < `num_long_metrics` have a `LongMetric`; any trailing
/// glyphs share the last advance and carry only a side-bearing in the
/// `left_side_bearings` array.
fn resolve_original_hmtx(
    hmtx: &write_fonts::read::tables::hmtx::Hmtx<'_>,
    gid: usize,
    num_long_metrics: usize,
) -> (u16, i16) {
    let long = hmtx.h_metrics();
    if gid < num_long_metrics && gid < long.len() {
        let m = &long[gid];
        (m.advance(), m.side_bearing())
    } else if !long.is_empty() {
        let last_advance = long[long.len() - 1].advance();
        let lsbs = hmtx.left_side_bearings();
        let lsb_idx = gid.saturating_sub(num_long_metrics);
        let lsb = lsbs.get(lsb_idx).map(|v| v.get()).unwrap_or(0);
        (last_advance, lsb)
    } else {
        (0, 0)
    }
}

/// Build a minimal GSUB table exposing `rand` feature backed by one
/// AlternateSubst lookup that covers every glyph with generated alternates.
fn build_gsub(alt_map: &[Vec<u16>]) -> Result<Gsub, String> {
    // Collect (origin_gid, alternates) pairs, sorted by gid so the coverage
    // array is in numerical order as required by CoverageFormat1.
    let mut pairs: Vec<(u16, &Vec<u16>)> = alt_map
        .iter()
        .enumerate()
        .filter_map(|(gid, alts)| {
            if alts.is_empty() {
                None
            } else {
                Some((gid as u16, alts))
            }
        })
        .collect();
    pairs.sort_by_key(|(gid, _)| *gid);

    // Degenerate case: nothing to alternate. Build a lookup-less feature so
    // downstream shapers still see a well-formed GSUB.
    if pairs.is_empty() {
        let lang_sys = LangSys::new(vec![]);
        let script = Script::new(Some(lang_sys), vec![]);
        let script_list = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"DFLT"), script)]);
        let feature_list = FeatureList::new(vec![]);
        let lookup_list: LookupList<SubstitutionLookup> = LookupList::new(vec![]);
        return Ok(Gsub::new(script_list, feature_list, lookup_list));
    }

    let coverage_glyphs: Vec<write_fonts::read::types::GlyphId16> = pairs
        .iter()
        .map(|(gid, _)| write_fonts::read::types::GlyphId16::new(*gid))
        .collect();
    let coverage = CoverageTable::Format1(CoverageFormat1::new(coverage_glyphs));

    let alt_sets: Vec<AlternateSet> = pairs
        .iter()
        .map(|(_, alts)| {
            AlternateSet::new(
                alts.iter()
                    .map(|gid| write_fonts::read::types::GlyphId16::new(*gid))
                    .collect(),
            )
        })
        .collect();

    let alt_subst = AlternateSubstFormat1::new(coverage, alt_sets);
    let lookup = Lookup::new(LookupFlag::empty(), vec![alt_subst]);
    let subst_lookup = SubstitutionLookup::Alternate(lookup);
    let lookup_list: LookupList<SubstitutionLookup> = LookupList::new(vec![subst_lookup]);

    let feature = Feature::new(None, vec![0]);
    let feature_record = FeatureRecord::new(Tag::new(b"rand"), feature);
    let feature_list = FeatureList::new(vec![feature_record]);

    let lang_sys = LangSys::new(vec![0]);
    let script = Script::new(Some(lang_sys), vec![]);
    let script_list = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"DFLT"), script)]);

    Ok(Gsub::new(script_list, feature_list, lookup_list))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_gsub_handles_empty() {
        let g = build_gsub(&[vec![], vec![]]).unwrap();
        // Script DFLT present, feature list empty, lookup list empty.
        assert_eq!(g.script_list.script_records.len(), 1);
        assert_eq!(g.feature_list.feature_records.len(), 0);
        assert_eq!(g.lookup_list.lookups.len(), 0);
    }

    #[test]
    fn build_gsub_with_one_alt_roundtrips() {
        // gid 1 has alternates 3 and 4.
        let mut map = vec![Vec::new(); 5];
        map[1] = vec![3, 4];
        let g = build_gsub(&map).unwrap();
        assert_eq!(
            g.feature_list.feature_records[0].feature_tag,
            Tag::new(b"rand")
        );
        assert_eq!(g.lookup_list.lookups.len(), 1);
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires macOS Arial.ttf; run with --ignored"]
    fn bake_arial_roundtrip() {
        let arial = std::path::Path::new("/System/Library/Fonts/Supplemental/Arial.ttf");
        if !arial.exists() {
            eprintln!("Skipping: Arial.ttf not found");
            return;
        }
        let tmp = std::env::temp_dir().join("jitter-bake-roundtrip.ttf");
        bake_font(arial, &tmp, 2, 0.3).expect("bake should succeed");
        let data = std::fs::read(&tmp).expect("read baked font");
        // skrifa must be able to parse it back.
        let file = skrifa::raw::FileRef::new(&data).expect("skrifa re-parse");
        let font = match file {
            skrifa::raw::FileRef::Font(f) => f,
            _ => panic!("expected single font"),
        };
        let maxp = font.maxp().expect("maxp");
        assert!(maxp.num_glyphs() > 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires macOS STIXGeneral.otf; run with --ignored"]
    fn bake_stix_otf_roundtrip() {
        let otf = std::path::Path::new("/System/Library/Fonts/Supplemental/STIXGeneral.otf");
        if !otf.exists() {
            eprintln!("Skipping: STIXGeneral.otf not found");
            return;
        }
        let tmp = std::env::temp_dir().join("jitter-bake-otf-roundtrip.ttf");
        bake_font(otf, &tmp, 2, 0.3).expect("bake should succeed for OTF input");
        let data = std::fs::read(&tmp).expect("read baked font");
        let file = skrifa::raw::FileRef::new(&data).expect("skrifa re-parse");
        let font = match file {
            skrifa::raw::FileRef::Font(f) => f,
            _ => panic!("expected single font"),
        };
        let maxp = font.maxp().expect("maxp");
        assert!(maxp.num_glyphs() > 0);
    }

    #[test]
    fn build_simple_glyph_with_cubic_produces_simple() {
        // A minimal cubic Bézier path that should convert to quadratic and produce a SimpleGlyph.
        let commands = vec![
            PathCommand::MoveTo(0.0, 0.0),
            PathCommand::CurveTo(10.0, 0.0, 20.0, 10.0, 30.0, 10.0),
            PathCommand::Close,
        ];
        let result = build_simple_glyph(&commands, 1).unwrap();
        assert!(
            matches!(result, Glyph::Simple(_)),
            "cubic commands should be approximated to quadratic and produce SimpleGlyph"
        );
    }

    #[test]
    fn build_simple_glyph_empty_returns_empty() {
        let result = build_simple_glyph(&[], 0).unwrap();
        assert!(matches!(result, Glyph::Empty));
    }
}
