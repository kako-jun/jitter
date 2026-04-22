//! Bake mode: generate a new TTF with alternate glyphs and a GSUB `calt` feature.
//!
//! Phase B scope:
//! - Input is TTF only (OTF / CFF / CFF2 is rejected).
//! - For each non-empty simple glyph (excluding `.notdef`), N alternates are
//!   created by re-running jitter, then registered in a minimal GSUB table
//!   using the `calt` feature (Contextual Alternates) with ChainContextSubst
//!   (LookupType 6, Format 1) and SingleSubstFormat2 lookups.
//! - `.notdef` (gid 0) is preserved unchanged and excluded from alternates.
//! - Composite glyphs are consumed via skrifa's pen and re-emitted as flat
//!   simple glyphs (structure flattened, visual appearance preserved).
//! - Cubic-bearing glyphs (non-TrueType outlines surfaced by skrifa) are
//!   emitted as empty placeholders without alternates; a summary warning is
//!   printed once after the pass.
//! - Most tables (name, OS/2, cmap, ...) are copied from the input font
//!   verbatim. `post` is downgraded to format 3.0 because the glyph count
//!   changed and the original format 2 glyph-name index would be stale.
//!
//! Shapers that honour the `calt` feature will cycle through alternates for
//! consecutive identical glyphs, producing the handwriting-like variation
//! baked into the font itself.
//!
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
use write_fonts::read::types::{GlyphId16, Tag};
use write_fonts::read::{FontRead, FontRef as WfFontRef};
use write_fonts::tables::glyf::{GlyfLocaBuilder, Glyph, SimpleGlyph};
use write_fonts::tables::gsub::{Gsub, SingleSubst, SubstitutionLookup};
use write_fonts::tables::head::Head;
use write_fonts::tables::hhea::Hhea;
use write_fonts::tables::hmtx::{Hmtx, LongMetric};
use write_fonts::tables::layout::{
    ChainedSequenceContext, ChainedSequenceRule, ChainedSequenceRuleSet, CoverageFormat1,
    CoverageTable, Feature, FeatureList, FeatureRecord, LangSys, Lookup, LookupFlag, LookupList,
    Script, ScriptList, ScriptRecord, SequenceLookupRecord,
};
use write_fonts::tables::maxp::Maxp;
use write_fonts::tables::post::Post;
use write_fonts::types::Version16Dot16;
use write_fonts::FontBuilder;

/// Bake jitter variation into a font file.
///
/// Reads the TTF at `input_path`, generates `alternates` jittered variants per
/// glyph, and writes a new TTF with the `calt` GSUB feature to `output_path`.
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

    // Reject OTF / CFF / CFF2. Phase A is TTF-only.
    if !is_ttf(&wf_font) {
        return Err("OTF/CFF fonts are not yet supported (TTF only)".to_string());
    }

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
    let mut cubic_warning_gids: Vec<u32> = Vec::new();
    for gid_u16 in 0..num_glyphs {
        let gid = GlyphId::new(gid_u16 as u32);

        let outline = outlines.get(gid);
        let (commands, is_simple) = if let Some(glyph) = outline {
            let mut pen = CollectPen::new();
            glyph
                .draw(Size::unscaled(), &mut pen)
                .map_err(|e| format!("Failed to draw glyph {gid_u16}: {e}"))?;
            (pen.commands, !pen.saw_nonsimple)
        } else {
            (Vec::new(), true)
        };

        if !is_simple {
            cubic_warning_gids.push(gid_u16 as u32);
        }

        originals.push(OriginalGlyph {
            commands,
            is_simple,
        });
    }

    // Emit a single summary warning for glyphs that contained cubic curves.
    // Those glyphs are written out as `Glyph::Empty` placeholders (keeping
    // their gid valid) and are not given any alternates.
    if !cubic_warning_gids.is_empty() {
        let first = cubic_warning_gids
            .iter()
            .take(5)
            .map(|g| g.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let more = if cubic_warning_gids.len() > 5 {
            format!(" (and {} more)", cubic_warning_gids.len() - 5)
        } else {
            String::new()
        };
        eprintln!(
            "warning: {} glyph(s) contained cubic curves and were emitted as empty placeholders without alternates: gid [{}]{}",
            cubic_warning_gids.len(),
            first,
            more
        );
    }

    // Compose the glyph list: originals first, then append alternates.
    // Track alternate gids per origin so we can build the GSUB calt
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
    let original_hmtx =
        write_fonts::read::tables::hmtx::Hmtx::read(hmtx_bytes_ro, num_long_metrics as u16)
            .map_err(|e| format!("Failed to read hmtx: {e}"))?;

    // Originals pass: build Glyph + LongMetric for each gid.
    // For cubic-bearing (is_simple=false) glyphs we still emit a placeholder
    // (Glyph::Empty) so the gid stays valid; the warning above surfaces this.
    for (gid, orig) in originals.iter().enumerate() {
        let glyph = build_simple_glyph(&orig.commands, orig.is_simple)?;
        new_glyphs.push(glyph);

        let (advance, lsb) = resolve_original_hmtx(&original_hmtx, gid, num_long_metrics);
        new_metrics.push(LongMetric::new(advance, lsb));
    }

    // Alternates pass: only for non-empty simple glyphs, and never for .notdef (gid 0).
    let mut next_gid: u32 = num_glyphs as u32;
    for gid in 0..num_glyphs as usize {
        if gid == 0 {
            // .notdef is the fallback glyph; do not vary it.
            continue;
        }
        let orig = &originals[gid];
        if !orig.is_simple || orig.commands.is_empty() {
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

            let glyph = build_simple_glyph(&jittered, true)?;
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

    // Build a minimal GSUB: script DFLT / langsys dflt / feature calt /
    // ChainContextSubst + SingleSubst lookups.
    let gsub = build_gsub_calt(&alt_map)?;

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
    let num_h_metrics = hhea.number_of_h_metrics();
    if num_h_metrics != expected_num_glyphs {
        return Err(format!(
            "bake produced inconsistent font: hhea.number_of_h_metrics={num_h_metrics}, expected {expected_num_glyphs}"
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

/// Detect whether a font is a TTF (has `glyf`, no `CFF ` / `CFF2`).
fn is_ttf(wf_font: &WfFontRef<'_>) -> bool {
    let has_glyf = wf_font.table_data(Tag::new(b"glyf")).is_some();
    let has_cff = wf_font.table_data(Tag::new(b"CFF ")).is_some()
        || wf_font.table_data(Tag::new(b"CFF2")).is_some();
    has_glyf && !has_cff
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
    /// Whether the glyph consists entirely of quadratic (TrueType) curves.
    /// False if the outline pen emitted any cubic curves, in which case the
    /// glyph is emitted as an empty placeholder and skipped for alternate
    /// generation.
    is_simple: bool,
}

/// OutlinePen that collects commands + notices constructs we can't re-emit as
/// a simple TTF glyph. Composite glyphs come through as drawn outlines via
/// skrifa, so we don't see a composite marker directly — we only flag cubic
/// segments, which are legal in CFF but not in the TTF glyf table.
struct CollectPen {
    commands: Vec<PathCommand>,
    saw_nonsimple: bool,
}

impl CollectPen {
    fn new() -> Self {
        Self {
            commands: Vec::new(),
            saw_nonsimple: false,
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
        // Cubic in a TTF input would mean the outline came from somewhere
        // exotic. We refuse to pretend, and skip alternate generation for
        // this glyph.
        self.saw_nonsimple = true;
        self.commands
            .push(PathCommand::CurveTo(cx0, cy0, cx1, cy1, x, y));
    }
    fn close(&mut self) {
        self.commands.push(PathCommand::Close);
    }
}

/// Convert a jitter path command list into a `write-fonts` SimpleGlyph via
/// kurbo. Empty commands -> empty glyph (glyf allows zero-length entries).
fn build_simple_glyph(commands: &[PathCommand], is_simple: bool) -> Result<Glyph, String> {
    if !is_simple || commands.is_empty() {
        return Ok(Glyph::Empty);
    }

    let mut path = BezPath::new();
    for cmd in commands {
        match *cmd {
            PathCommand::MoveTo(x, y) => path.move_to((x as f64, y as f64)),
            PathCommand::LineTo(x, y) => path.line_to((x as f64, y as f64)),
            PathCommand::QuadTo(cx, cy, x, y) => {
                path.quad_to((cx as f64, cy as f64), (x as f64, y as f64))
            }
            PathCommand::CurveTo(cx0, cy0, cx1, cy1, x, y) => path.curve_to(
                (cx0 as f64, cy0 as f64),
                (cx1 as f64, cy1 as f64),
                (x as f64, y as f64),
            ),
            PathCommand::Close => path.close_path(),
        }
    }

    match SimpleGlyph::from_bezpath(&path) {
        Ok(g) => Ok(Glyph::Simple(g)),
        Err(_) => Ok(Glyph::Empty),
    }
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

/// Build a minimal GSUB table exposing `calt` feature backed by
/// ChainContextSubst lookups that cycle through alternates for consecutive
/// identical glyphs.
///
/// For each origin with N alternates, we create:
/// - N SingleSubst lookups: origin → alt_k (k = 0..N-1)
/// - 1 ChainContextSubst lookup: Coverage=[origin], with N+1 rules that
///   check the backtrack glyph and apply the appropriate SingleSubst.
///
/// Chain rules:
///   backtrack=[origin] → SingleSubst_0 (origin→alt_0)
///   backtrack=[alt_0]  → SingleSubst_1 (origin→alt_1)
///   ...
///   backtrack=[alt_{N-2}] → SingleSubst_{N-1} (origin→alt_{N-1})
///   backtrack=[alt_{N-1}] → SingleSubst_0 (origin→alt_0)  (cycle)
fn build_gsub_calt(alt_map: &[Vec<u16>]) -> Result<Gsub, String> {
    // Collect (origin_gid, alternates) pairs, sorted by gid so lookups
    // are built in a stable, deterministic order.
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

    let mut lookups: Vec<SubstitutionLookup> = Vec::new();
    let mut feature_lookup_indices: Vec<u16> = Vec::new();

    for (origin_gid, alts) in &pairs {
        let num_alts = alts.len();
        if num_alts == 0 {
            continue;
        }

        // Base index in the global LookupList for this origin's SingleSubst lookups.
        let base_lookup_idx = lookups.len() as u16;

        // Create N SingleSubst lookups: origin → alt_k
        for alt_gid in alts.iter() {
            let coverage =
                CoverageTable::Format1(CoverageFormat1::new(vec![GlyphId16::new(*origin_gid)]));
            let subst = SingleSubst::format_2(coverage, vec![GlyphId16::new(*alt_gid)]);
            let lookup = Lookup::new(LookupFlag::empty(), vec![subst]);
            lookups.push(SubstitutionLookup::Single(lookup));
        }

        // Create 1 ChainContextSubst lookup for this origin.
        let coverage =
            CoverageTable::Format1(CoverageFormat1::new(vec![GlyphId16::new(*origin_gid)]));

        let mut rules: Vec<ChainedSequenceRule> = Vec::with_capacity(num_alts + 1);

        // Rule 0: previous glyph is origin → apply SingleSubst_0 (origin→alt_0)
        rules.push(ChainedSequenceRule::new(
            vec![GlyphId16::new(*origin_gid)],
            vec![],
            vec![],
            vec![SequenceLookupRecord::new(0, base_lookup_idx)],
        ));

        // Rules 1..N-1: previous glyph is alt_{k-1} → apply SingleSubst_k
        for k in 1..num_alts {
            rules.push(ChainedSequenceRule::new(
                vec![GlyphId16::new(alts[k - 1])],
                vec![],
                vec![],
                vec![SequenceLookupRecord::new(0, base_lookup_idx + k as u16)],
            ));
        }

        // Rule N: previous glyph is alt_{N-1} → apply SingleSubst_0 (cycle back)
        rules.push(ChainedSequenceRule::new(
            vec![GlyphId16::new(alts[num_alts - 1])],
            vec![],
            vec![],
            vec![SequenceLookupRecord::new(0, base_lookup_idx)],
        ));

        let rule_set = ChainedSequenceRuleSet::new(rules);
        let chain_context = ChainedSequenceContext::format_1(coverage, vec![Some(rule_set)]);
        let chain_lookup = Lookup::new(LookupFlag::empty(), vec![chain_context.into()]);
        let chain_lookup_idx = lookups.len() as u16;
        lookups.push(SubstitutionLookup::ChainContextual(chain_lookup));
        feature_lookup_indices.push(chain_lookup_idx);
    }

    let feature = Feature::new(None, feature_lookup_indices);
    let feature_record = FeatureRecord::new(Tag::new(b"calt"), feature);
    let feature_list = FeatureList::new(vec![feature_record]);

    let lang_sys = LangSys::new(vec![0]);
    let script = Script::new(Some(lang_sys), vec![]);
    let script_list = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"DFLT"), script)]);

    let lookup_list = LookupList::new(lookups);

    Ok(Gsub::new(script_list, feature_list, lookup_list))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_ttf_errors_on_garbage() {
        let data = b"not a font at all";
        assert!(WfFontRef::new(data.as_slice()).is_err());
    }

    #[test]
    fn build_gsub_calt_handles_empty() {
        let g = build_gsub_calt(&[vec![], vec![]]).unwrap();
        // Script DFLT present, feature list empty, lookup list empty.
        assert_eq!(g.script_list.script_records.len(), 1);
        assert_eq!(g.feature_list.feature_records.len(), 0);
        assert_eq!(g.lookup_list.lookups.len(), 0);
    }

    #[test]
    fn build_gsub_calt_with_one_alt_roundtrips() {
        // gid 1 has alternates 3 and 4 (N = 2).
        // Expect 2 SingleSubst + 1 ChainContextSubst = 3 lookups.
        let mut map = vec![Vec::new(); 5];
        map[1] = vec![3, 4];
        let g = build_gsub_calt(&map).unwrap();
        assert_eq!(
            g.feature_list.feature_records[0].feature_tag,
            Tag::new(b"calt")
        );
        assert_eq!(g.lookup_list.lookups.len(), 3);
    }

    #[test]
    fn build_gsub_calt_with_single_alt() {
        // N = 1: 1 SingleSubst + 1 ChainContextSubst (2 rules: origin→alt, alt→alt cycle).
        let mut map = vec![Vec::new(); 5];
        map[1] = vec![3];
        let g = build_gsub_calt(&map).unwrap();
        assert_eq!(
            g.feature_list.feature_records[0].feature_tag,
            Tag::new(b"calt")
        );
        assert_eq!(g.lookup_list.lookups.len(), 2);
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
}
