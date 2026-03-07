use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};
use serde::Serialize;
use text_chunker::*;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "text-chunker", about = "Split page-marked documents into chunks")]
pub struct Cli {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Show summary table of all pages
    Pages {
        /// Input file path, or "-" for stdin
        file: String,
    },
    /// Show non-empty lines of a page or page range (e.g. 5 or 5-10)
    Lines {
        /// Input file path, or "-" for stdin
        file: String,
        /// Page number or range (e.g. "5" or "5-10")
        page: String,
        /// Output all original lines including the page marker
        #[arg(long)]
        raw: bool,
        /// Output all lines except the page marker comment (blanks preserved)
        #[arg(long)]
        no_markers: bool,
    },
    /// Search for a term across all pages
    Search {
        /// Input file path, or "-" for stdin
        file: String,
        /// Search term (case-insensitive)
        term: String,
    },
    /// Split pages into individual files
    Split {
        /// Input file path, or "-" for stdin
        file: String,
        /// Output directory
        #[arg(long)]
        outdir: PathBuf,
    },
    /// Start MCP (Model Context Protocol) server over stdio
    Mcp,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        process::exit(1);
    }
}

fn read_input(file: &str) -> Result<String, ChunkerError> {
    if file == "-" {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        Ok(buf)
    } else {
        validate_extension(file)?;
        Ok(std::fs::read_to_string(file)?)
    }
}

fn run() -> Result<(), ChunkerError> {
    let cli = Cli::parse();

    if let Commands::Mcp = &cli.command {
        let rt = tokio::runtime::Runtime::new().map_err(ChunkerError::Io)?;
        rt.block_on(text_chunker::mcp::run_mcp_server())
            .map_err(|e| ChunkerError::Io(std::io::Error::other(e.to_string())))?;
        return Ok(());
    }

    let file = match &cli.command {
        Commands::Pages { file }
        | Commands::Lines { file, .. }
        | Commands::Search { file, .. }
        | Commands::Split { file, .. } => file.as_str(),
        Commands::Mcp => unreachable!(),
    };
    let content = read_input(file)?;
    let pages = parse_pages(&content)?;

    match cli.command {
        Commands::Pages { .. } => {
            if cli.json {
                println!("{}", pages_to_json(&pages));
            } else {
                print!("{}", format_pages_table(&pages));
            }
        }
        Commands::Lines { page, raw, no_markers, .. } => {
            let (start, end) = parse_page_range(&page)?;
            let mode = if raw {
                LineMode::Raw
            } else if no_markers {
                LineMode::NoMarkers
            } else {
                LineMode::Content
            };
            let result = collect_page_lines(&pages, start, end, mode)?;
            if cli.json {
                println!("{}", lines_to_json(&result.lines, start, end, result.line_start, result.line_end));
            } else {
                for line in &result.lines {
                    println!("{}", line);
                }
            }
        }
        Commands::Search { term, .. } => {
            let matches = search_pages(&pages, &term);
            if cli.json {
                println!("{}", search_to_json(&matches, &term));
            } else {
                print!("{}", format_search_results(&matches));
            }
        }
        Commands::Split { outdir, .. } => {
            split_pages(&pages, &outdir)?;
            if cli.json {
                #[derive(Serialize)]
                struct SplitResult {
                    pages_written: usize,
                    outdir: String,
                }
                let result = SplitResult {
                    pages_written: pages.len(),
                    outdir: outdir.display().to_string(),
                };
                println!("{}", serde_json::to_string_pretty(&result).unwrap());
            } else {
                println!(
                    "Wrote {} pages to {}",
                    pages.len(),
                    outdir.display()
                );
            }
        }
        Commands::Mcp => unreachable!(),
    }

    Ok(())
}
