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
        #[arg(short, long, default_value = "0.5")]
        intensity: f64,

        /// Font size in pixels
        #[arg(short, long, default_value = "48")]
        size: u32,
    },
    /// Bake variation into a font file (generates calt alternates)
    Bake {
        /// Input font file (.ttf or .otf)
        input: PathBuf,

        /// Output font file path
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Number of alternate glyphs per character
        #[arg(short, long, default_value = "3")]
        alternates: u32,

        /// Variation intensity (0.0 to 1.0)
        #[arg(short, long, default_value = "0.5")]
        intensity: f64,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Render {
            text,
            font,
            output,
            intensity,
            size,
        } => {
            println!(
                "Rendering \"{}\" with font {:?} (size: {}, intensity: {}) -> {:?}",
                text,
                font.display(),
                size,
                intensity,
                output.display()
            );
            eprintln!("render mode is not yet implemented");
            std::process::exit(1);
        }
        Commands::Bake {
            input,
            output,
            alternates,
            intensity,
        } => {
            let output = output.unwrap_or_else(|| {
                let stem = input.file_stem().unwrap().to_str().unwrap();
                let ext = input.extension().unwrap().to_str().unwrap();
                input.with_file_name(format!("{stem}-jittered.{ext}"))
            });
            println!(
                "Baking {:?} with {} alternates (intensity: {}) -> {:?}",
                input.display(),
                alternates,
                intensity,
                output.display()
            );
            eprintln!("bake mode is not yet implemented");
            std::process::exit(1);
        }
    }
}
