mod bake;
mod font;
mod jitter;
mod layout;
mod png;
mod svg;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "jitter")]
#[command(about = "Add natural handwriting-like variation to digital text")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Render text with per-character random variation
    Render {
        /// Text to render
        text: String,

        /// Path to font file (.ttf or .otf)
        #[arg(short, long)]
        font: PathBuf,

        /// Output file path (.svg or .png)
        #[arg(short, long, default_value = "output.svg")]
        output: PathBuf,

        /// Variation intensity (0.0 to 1.0)
        #[arg(short, long, default_value = "0.5", value_parser = parse_intensity)]
        intensity: f64,

        /// Font size in pixels
        #[arg(short, long, default_value = "48")]
        size: u32,

        /// Random seed for reproducible output (u64)
        #[arg(long, value_name = "N")]
        seed: Option<u64>,
    },
    /// Bake variation into a font file (generates calt alternates)
    Bake {
        /// Input font file (.ttf or .otf)
        input: PathBuf,

        /// Output font file path
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Number of alternate glyphs per character
        #[arg(short, long, default_value = "3", value_parser = parse_alternates)]
        alternates: u32,

        /// Variation intensity (0.0 to 1.0)
        #[arg(short, long, default_value = "0.5", value_parser = parse_intensity)]
        intensity: f64,
    },
}

fn parse_alternates(s: &str) -> Result<u32, String> {
    let v: u32 = s.parse().map_err(|e| format!("{e}"))?;
    if v >= 1 {
        Ok(v)
    } else {
        Err("alternates must be at least 1".to_string())
    }
}

fn parse_intensity(s: &str) -> Result<f64, String> {
    let v: f64 = s.parse().map_err(|e| format!("{e}"))?;
    if (0.0..=1.0).contains(&v) {
        Ok(v)
    } else {
        Err(format!("intensity must be between 0.0 and 1.0, got {v}"))
    }
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Render {
            text,
            font: font_path,
            output,
            intensity,
            size,
            seed,
        } => {
            if text.is_empty() {
                eprintln!("Error: text must not be empty");
                std::process::exit(1);
            }

            let (glyphs, units_per_em) = match font::load_glyphs(&font_path, &text) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            };

            let commands: Vec<Vec<font::PathCommand>> =
                glyphs.iter().map(|g| g.commands.clone()).collect();
            let jittered = jitter::apply_jitter(&commands, intensity, units_per_em as f64, seed);

            let seed_note = match seed {
                Some(s) => format!(" (seed: {s})"),
                None => String::new(),
            };

            let ext = output.extension();
            let is_svg = ext.is_none() || ext.is_some_and(|e| e.eq_ignore_ascii_case("svg"));
            let is_png = ext.is_some_and(|e| e.eq_ignore_ascii_case("png"));

            if is_svg {
                let svg_output = svg::render_svg(&glyphs, &jittered, size, units_per_em);

                if let Err(e) = std::fs::write(&output, &svg_output) {
                    eprintln!("Error writing output: {e}");
                    std::process::exit(1);
                }

                println!(
                    "Rendered \"{}\" -> {} ({} bytes){}",
                    text,
                    output.display(),
                    svg_output.len(),
                    seed_note
                );
            } else if is_png {
                let png_bytes = match png::render_png(&glyphs, &jittered, size, units_per_em) {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("Error rendering PNG: {e}");
                        std::process::exit(1);
                    }
                };

                if let Err(e) = std::fs::write(&output, &png_bytes) {
                    eprintln!("Error writing output: {e}");
                    std::process::exit(1);
                }

                println!(
                    "Rendered \"{}\" -> {} ({} bytes){}",
                    text,
                    output.display(),
                    png_bytes.len(),
                    seed_note
                );
            } else {
                let lossy = ext
                    .map(|e| e.to_string_lossy().into_owned())
                    .unwrap_or_default();
                eprintln!("Error: unsupported output extension: .{lossy} (supported: .svg, .png)");
                std::process::exit(1);
            }
        }
        Commands::Bake {
            input,
            output,
            alternates,
            intensity,
        } => {
            let output = output.unwrap_or_else(|| {
                let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("font");
                let ext = input.extension().and_then(|s| s.to_str()).unwrap_or("ttf");
                input.with_file_name(format!("{stem}-jittered.{ext}"))
            });
            println!(
                "Baking {} with {} alternates (intensity: {}) -> {}",
                input.display(),
                alternates,
                intensity,
                output.display()
            );
            if let Err(e) = bake::bake_font(&input, &output, alternates, intensity) {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
            println!("Wrote {}", output.display());
        }
    }
}
