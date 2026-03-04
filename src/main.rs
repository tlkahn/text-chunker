use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};
use regex::Regex;
use serde::Serialize;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ChunkerError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("file is empty")]
    EmptyFile,
    #[error("no page markers found")]
    NoPagesFound,
    #[error("page {0} not found")]
    PageNotFound(usize),
    #[error("unsupported file extension: {0}")]
    UnsupportedExtension(String),
    #[error("invalid page range: {0}")]
    InvalidPageRange(String),
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Page {
    pub number: usize,
    pub image_count: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub lines: Vec<String>,
}

impl Page {
    pub fn content_lines(&self) -> Vec<&str> {
        self.lines
            .iter()
            .skip(1)
            .map(|s| s.as_str())
            .filter(|s| !s.trim().is_empty())
            .collect()
    }

    pub fn content_line_count(&self) -> usize {
        self.content_lines().len()
    }

    pub fn raw_lines(&self) -> &[String] {
        &self.lines
    }

    pub fn raw_lines_sans_marker(&self) -> &[String] {
        if self.lines.is_empty() { &self.lines } else { &self.lines[1..] }
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

pub fn parse_pages(content: &str) -> Result<Vec<Page>, ChunkerError> {
    if content.trim().is_empty() {
        return Err(ChunkerError::EmptyFile);
    }

    let re = Regex::new(r"<!--\s*Page\s+(\d+)\s*-\s*(\d+)\s+images?\s*-->").unwrap();
    let all_lines: Vec<&str> = content.lines().collect();

    let mut markers: Vec<(usize, usize, usize)> = Vec::new();
    for (i, line) in all_lines.iter().enumerate() {
        if let Some(caps) = re.captures(line) {
            let page_num: usize = caps[1].parse().unwrap();
            let img_count: usize = caps[2].parse().unwrap();
            markers.push((i, page_num, img_count));
        }
    }

    if markers.is_empty() {
        return Err(ChunkerError::NoPagesFound);
    }

    let mut pages = Vec::new();
    for (idx, &(line_idx, page_num, img_count)) in markers.iter().enumerate() {
        let end_idx = if idx + 1 < markers.len() {
            markers[idx + 1].0
        } else {
            all_lines.len()
        };
        let lines: Vec<String> = all_lines[line_idx..end_idx]
            .iter()
            .map(|s| s.to_string())
            .collect();
        pages.push(Page {
            number: page_num,
            image_count: img_count,
            start_line: line_idx + 1,
            end_line: end_idx,
            lines,
        });
    }

    Ok(pages)
}

pub fn parse_page_range(spec: &str) -> Result<(usize, usize), ChunkerError> {
    if let Some((left, right)) = spec.split_once('-') {
        let start: usize = left
            .trim()
            .parse()
            .map_err(|_| ChunkerError::InvalidPageRange(spec.to_string()))?;
        let end: usize = right
            .trim()
            .parse()
            .map_err(|_| ChunkerError::InvalidPageRange(spec.to_string()))?;
        if start > end {
            return Err(ChunkerError::InvalidPageRange(spec.to_string()));
        }
        Ok((start, end))
    } else {
        let n: usize = spec
            .trim()
            .parse()
            .map_err(|_| ChunkerError::InvalidPageRange(spec.to_string()))?;
        Ok((n, n))
    }
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct SearchMatch {
    pub page_number: usize,
    pub line: String,
}

pub fn search_pages(pages: &[Page], term: &str) -> Vec<SearchMatch> {
    let term_lower = term.to_lowercase();
    let mut results = Vec::new();
    for page in pages {
        for line in page.content_lines() {
            if line.to_lowercase().contains(&term_lower) {
                results.push(SearchMatch {
                    page_number: page.number,
                    line: line.to_string(),
                });
            }
        }
    }
    results
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

pub fn format_pages_table(pages: &[Page]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{:<8} {:<8} {:<14} {}\n",
        "Page", "Images", "Lines", "Content"
    ));
    out.push_str(&format!("{}\n", "-".repeat(46)));

    let mut total_content = 0usize;
    for p in pages {
        let cc = p.content_line_count();
        total_content += cc;
        let content_str = if cc == 0 {
            "[empty]".to_string()
        } else {
            cc.to_string()
        };
        out.push_str(&format!(
            "{:<8} {:<8} {:<14} {}\n",
            p.number,
            p.image_count,
            format!("{}-{}", p.start_line, p.end_line),
            content_str,
        ));
    }

    out.push_str(&format!("{}\n", "-".repeat(46)));
    out.push_str(&format!(
        "Total: {} pages, {} content lines\n",
        pages.len(),
        total_content
    ));
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineMode {
    Content,
    Raw,
    NoMarkers,
}

pub fn collect_page_lines(
    pages: &[Page],
    start: usize,
    end: usize,
    mode: LineMode,
) -> Result<Vec<String>, ChunkerError> {
    let mut lines = Vec::new();
    for num in start..=end {
        let page = pages
            .iter()
            .find(|p| p.number == num)
            .ok_or(ChunkerError::PageNotFound(num))?;
        match mode {
            LineMode::Content => {
                lines.extend(page.content_lines().into_iter().map(|s| s.to_string()));
            }
            LineMode::Raw => {
                lines.extend_from_slice(page.raw_lines());
            }
            LineMode::NoMarkers => {
                lines.extend_from_slice(page.raw_lines_sans_marker());
            }
        }
    }
    Ok(lines)
}

pub fn format_search_results(matches: &[SearchMatch]) -> String {
    if matches.is_empty() {
        return "No matches found.\n".to_string();
    }
    let mut out = String::new();
    let mut current_page: Option<usize> = None;
    for m in matches {
        if current_page != Some(m.page_number) {
            out.push_str(&format!("\nPage {}:\n", m.page_number));
            current_page = Some(m.page_number);
        }
        out.push_str(&format!("  {}\n", m.line));
    }
    out
}

pub fn split_pages(pages: &[Page], outdir: &std::path::Path) -> Result<(), ChunkerError> {
    std::fs::create_dir_all(outdir)?;
    for page in pages {
        let filename = format!("page-{:03}.md", page.number);
        let path = outdir.join(filename);
        let content: String = page
            .content_lines()
            .join("\n");
        std::fs::write(&path, content)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// JSON output
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct PageSummary {
    pub number: usize,
    pub image_count: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub content_line_count: usize,
    pub is_empty: bool,
}

#[derive(Debug, Serialize)]
pub struct PagesJson {
    pub pages: Vec<PageSummary>,
    pub total_pages: usize,
    pub total_content_lines: usize,
}

pub fn pages_to_json(pages: &[Page]) -> String {
    let summaries: Vec<PageSummary> = pages
        .iter()
        .map(|p| {
            let cc = p.content_line_count();
            PageSummary {
                number: p.number,
                image_count: p.image_count,
                start_line: p.start_line,
                end_line: p.end_line,
                content_line_count: cc,
                is_empty: cc == 0,
            }
        })
        .collect();
    let total_content: usize = summaries.iter().map(|s| s.content_line_count).sum();
    let output = PagesJson {
        total_pages: summaries.len(),
        total_content_lines: total_content,
        pages: summaries,
    };
    serde_json::to_string_pretty(&output).unwrap()
}

pub fn lines_to_json(lines: &[String], start: usize, end: usize) -> String {
    #[derive(Serialize)]
    struct LinesJson {
        page_start: usize,
        page_end: usize,
        count: usize,
        lines: Vec<String>,
    }
    let output = LinesJson {
        page_start: start,
        page_end: end,
        count: lines.len(),
        lines: lines.to_vec(),
    };
    serde_json::to_string_pretty(&output).unwrap()
}

pub fn search_to_json(matches: &[SearchMatch], term: &str) -> String {
    #[derive(Serialize)]
    struct SearchJson<'a> {
        term: &'a str,
        total_matches: usize,
        matches: &'a [SearchMatch],
    }
    let output = SearchJson {
        term,
        total_matches: matches.len(),
        matches,
    };
    serde_json::to_string_pretty(&output).unwrap()
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

pub fn validate_extension(path: &str) -> Result<(), ChunkerError> {
    let p = std::path::Path::new(path);
    match p.extension().and_then(|e| e.to_str()) {
        Some("md" | "txt") => Ok(()),
        Some(ext) => Err(ChunkerError::UnsupportedExtension(ext.to_string())),
        None => Err(ChunkerError::UnsupportedExtension("(none)".to_string())),
    }
}

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

    let file = match &cli.command {
        Commands::Pages { file }
        | Commands::Lines { file, .. }
        | Commands::Search { file, .. }
        | Commands::Split { file, .. } => file.as_str(),
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
            let lines = collect_page_lines(&pages, start, end, mode)?;
            if cli.json {
                println!("{}", lines_to_json(&lines, start, end));
            } else {
                for line in &lines {
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
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
<!-- Page 0 - 2 images -->
First page line one
First page line two

<!-- Page 1 - 3 images -->
Second page content
<!-- Page 2 - 1 image -->
Third page only line";

    // -- parse_pages basic ---------------------------------------------------

    #[test]
    fn test_parse_page_count() {
        let pages = parse_pages(SAMPLE).unwrap();
        assert_eq!(pages.len(), 3);
    }

    #[test]
    fn test_parse_page_numbers() {
        let pages = parse_pages(SAMPLE).unwrap();
        let nums: Vec<usize> = pages.iter().map(|p| p.number).collect();
        assert_eq!(nums, vec![0, 1, 2]);
    }

    #[test]
    fn test_parse_image_counts() {
        let pages = parse_pages(SAMPLE).unwrap();
        let imgs: Vec<usize> = pages.iter().map(|p| p.image_count).collect();
        assert_eq!(imgs, vec![2, 3, 1]);
    }

    #[test]
    fn test_singular_image_marker() {
        let input = "<!-- Page 5 - 1 image -->\nsome line";
        let pages = parse_pages(input).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].image_count, 1);
    }

    // -- content_lines -------------------------------------------------------

    #[test]
    fn test_content_lines_skip_marker_and_blanks() {
        let pages = parse_pages(SAMPLE).unwrap();
        let cl = pages[0].content_lines();
        assert_eq!(cl, vec!["First page line one", "First page line two"]);
    }

    #[test]
    fn test_content_line_count() {
        let pages = parse_pages(SAMPLE).unwrap();
        assert_eq!(pages[0].content_line_count(), 2);
        assert_eq!(pages[1].content_line_count(), 1);
        assert_eq!(pages[2].content_line_count(), 1);
    }

    // -- line ranges ---------------------------------------------------------

    #[test]
    fn test_start_line_numbers() {
        let pages = parse_pages(SAMPLE).unwrap();
        assert_eq!(pages[0].start_line, 1);
        assert_eq!(pages[1].start_line, 5);
    }

    // -- error cases ---------------------------------------------------------

    #[test]
    fn test_empty_file_error() {
        let result = parse_pages("");
        assert!(matches!(result, Err(ChunkerError::EmptyFile)));
    }

    #[test]
    fn test_whitespace_only_file_error() {
        let result = parse_pages("   \n  \n  ");
        assert!(matches!(result, Err(ChunkerError::EmptyFile)));
    }

    #[test]
    fn test_no_markers_error() {
        let result = parse_pages("just some text\nwithout markers");
        assert!(matches!(result, Err(ChunkerError::NoPagesFound)));
    }

    // -- parse_page_range ----------------------------------------------------

    #[test]
    fn test_parse_single_page() {
        let (start, end) = parse_page_range("5").unwrap();
        assert_eq!((start, end), (5, 5));
    }

    #[test]
    fn test_parse_page_range_pair() {
        let (start, end) = parse_page_range("5-10").unwrap();
        assert_eq!((start, end), (5, 10));
    }

    #[test]
    fn test_parse_page_range_invalid() {
        let result = parse_page_range("abc");
        assert!(matches!(result, Err(ChunkerError::InvalidPageRange(_))));
    }

    #[test]
    fn test_parse_page_range_reversed() {
        let result = parse_page_range("10-5");
        assert!(matches!(result, Err(ChunkerError::InvalidPageRange(_))));
    }

    // -- search --------------------------------------------------------------

    #[test]
    fn test_search_finds_matching_lines() {
        let pages = parse_pages(SAMPLE).unwrap();
        let results = search_pages(&pages, "page line");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].page_number, 0);
        assert!(results[0].line.contains("page line"));
    }

    #[test]
    fn test_search_case_insensitive() {
        let pages = parse_pages(SAMPLE).unwrap();
        let results = search_pages(&pages, "FIRST");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_search_no_results() {
        let pages = parse_pages(SAMPLE).unwrap();
        let results = search_pages(&pages, "zzzznotfound");
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_excludes_marker_lines() {
        let pages = parse_pages(SAMPLE).unwrap();
        let results = search_pages(&pages, "images");
        assert!(results.is_empty());
    }

    // -- format_pages_table --------------------------------------------------

    #[test]
    fn test_pages_table_contains_header() {
        let pages = parse_pages(SAMPLE).unwrap();
        let table = format_pages_table(&pages);
        assert!(table.contains("Page"));
        assert!(table.contains("Images"));
        assert!(table.contains("Content"));
    }

    #[test]
    fn test_pages_table_contains_all_pages() {
        let pages = parse_pages(SAMPLE).unwrap();
        let table = format_pages_table(&pages);
        // Should contain entries for page 0, 1, 2
        for p in &pages {
            assert!(table.contains(&format!("{}", p.number)));
        }
    }

    #[test]
    fn test_pages_table_marks_empty_pages() {
        let input = "<!-- Page 0 - 1 image -->\n   \n\n<!-- Page 1 - 2 images -->\ncontent";
        let pages = parse_pages(input).unwrap();
        let table = format_pages_table(&pages);
        assert!(table.contains("[empty]"));
    }

    #[test]
    fn test_pages_table_has_total_row() {
        let pages = parse_pages(SAMPLE).unwrap();
        let table = format_pages_table(&pages);
        assert!(table.contains("Total"));
    }

    // -- collect_page_lines --------------------------------------------------

    #[test]
    fn test_collect_single_page_lines() {
        let pages = parse_pages(SAMPLE).unwrap();
        let lines = collect_page_lines(&pages, 0, 0, LineMode::Content).unwrap();
        assert_eq!(lines, vec!["First page line one", "First page line two"]);
    }

    #[test]
    fn test_collect_page_range_lines() {
        let pages = parse_pages(SAMPLE).unwrap();
        let lines = collect_page_lines(&pages, 0, 1, LineMode::Content).unwrap();
        assert_eq!(
            lines,
            vec![
                "First page line one",
                "First page line two",
                "Second page content"
            ]
        );
    }

    #[test]
    fn test_collect_page_not_found() {
        let pages = parse_pages(SAMPLE).unwrap();
        let result = collect_page_lines(&pages, 99, 99, LineMode::Content);
        assert!(matches!(result, Err(ChunkerError::PageNotFound(99))));
    }

    // -- format_search_results -----------------------------------------------

    #[test]
    fn test_format_search_results_contains_page_nums() {
        let pages = parse_pages(SAMPLE).unwrap();
        let matches = search_pages(&pages, "page line");
        let output = format_search_results(&matches);
        assert!(output.contains("Page 0"));
    }

    #[test]
    fn test_format_search_results_empty() {
        let output = format_search_results(&[]);
        assert!(output.contains("No matches"));
    }

    // -- JSON output ---------------------------------------------------------

    #[test]
    fn test_pages_json_is_valid() {
        let pages = parse_pages(SAMPLE).unwrap();
        let json_str = pages_to_json(&pages);
        let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["total_pages"], 3);
        assert_eq!(v["pages"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_pages_json_marks_empty() {
        let input = "<!-- Page 0 - 1 image -->\n   \n\n<!-- Page 1 - 2 images -->\ncontent";
        let pages = parse_pages(input).unwrap();
        let json_str = pages_to_json(&pages);
        let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["pages"][0]["is_empty"], true);
        assert_eq!(v["pages"][1]["is_empty"], false);
    }

    #[test]
    fn test_lines_json_is_valid() {
        let pages = parse_pages(SAMPLE).unwrap();
        let lines = collect_page_lines(&pages, 0, 0, LineMode::Content).unwrap();
        let json_str = lines_to_json(&lines, 0, 0);
        let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["lines"].as_array().unwrap().len(), 2);
        assert_eq!(v["page_start"], 0);
        assert_eq!(v["page_end"], 0);
    }

    #[test]
    fn test_search_json_is_valid() {
        let pages = parse_pages(SAMPLE).unwrap();
        let matches = search_pages(&pages, "page line");
        let json_str = search_to_json(&matches, "page line");
        let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["term"], "page line");
        assert_eq!(v["total_matches"], 2);
        assert!(v["matches"].as_array().unwrap().len() == 2);
    }

    // -- validate_extension --------------------------------------------------

    #[test]
    fn test_validate_md_extension() {
        assert!(validate_extension("file.md").is_ok());
    }

    #[test]
    fn test_validate_txt_extension() {
        assert!(validate_extension("file.txt").is_ok());
    }

    #[test]
    fn test_validate_bad_extension() {
        let result = validate_extension("file.pdf");
        assert!(matches!(result, Err(ChunkerError::UnsupportedExtension(_))));
    }

    // -- blank page detection ------------------------------------------------

    #[test]
    fn test_blank_page_detected() {
        let input = "<!-- Page 0 - 1 image -->\n   \n\n<!-- Page 1 - 2 images -->\nreal content";
        let pages = parse_pages(input).unwrap();
        assert_eq!(pages[0].content_line_count(), 0);
        assert_eq!(pages[1].content_line_count(), 1);
    }

    // -- edge cases ----------------------------------------------------------

    // #1 Content before first marker is silently dropped
    #[test]
    fn test_content_before_first_marker_dropped() {
        let input = "preamble line\nanother line\n<!-- Page 0 - 1 image -->\nreal content";
        let pages = parse_pages(input).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].number, 0);
        let cl = pages[0].content_lines();
        assert_eq!(cl, vec!["real content"]);
        assert!(!cl.iter().any(|l| l.contains("preamble")));
    }

    // #2 Non-sequential page numbers — range query hits gap
    #[test]
    fn test_non_sequential_pages_range_gap() {
        let input = "<!-- Page 0 - 1 image -->\nline a\n<!-- Page 5 - 1 image -->\nline b\n<!-- Page 10 - 1 image -->\nline c";
        let pages = parse_pages(input).unwrap();
        assert_eq!(pages.len(), 3);
        // Range 0-10 should fail because pages 1-4 don't exist
        let result = collect_page_lines(&pages, 0, 10, LineMode::Content);
        assert!(matches!(result, Err(ChunkerError::PageNotFound(1))));
    }

    // #3 Duplicate page numbers — find returns first
    #[test]
    fn test_duplicate_page_numbers() {
        let input = "<!-- Page 3 - 1 image -->\nfirst version\n<!-- Page 3 - 2 images -->\nsecond version";
        let pages = parse_pages(input).unwrap();
        assert_eq!(pages.len(), 2);
        // Both have number 3
        assert_eq!(pages[0].number, 3);
        assert_eq!(pages[1].number, 3);
        // collect_page_lines finds the first one
        let lines = collect_page_lines(&pages, 3, 3, LineMode::Content).unwrap();
        assert_eq!(lines, vec!["first version"]);
    }

    // #4 Consecutive markers with no content between them
    #[test]
    fn test_consecutive_markers_no_content() {
        let input = "<!-- Page 0 - 1 image -->\n<!-- Page 1 - 1 image -->\nsome content";
        let pages = parse_pages(input).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].content_line_count(), 0);
        assert_eq!(pages[1].content_line_count(), 1);
    }

    // #5 Windows line endings (\r\n) — content lines should be clean
    #[test]
    fn test_windows_line_endings() {
        let input = "<!-- Page 0 - 1 image -->\r\ncontent with cr\r\n\r\n<!-- Page 1 - 1 image -->\r\nsecond page";
        let pages = parse_pages(input).unwrap();
        assert_eq!(pages.len(), 2);
        let cl = pages[0].content_lines();
        // Should not have trailing \r
        assert_eq!(cl, vec!["content with cr"]);
        assert!(!cl[0].ends_with('\r'));
    }

    // #6 Zero images
    #[test]
    fn test_zero_images() {
        let input = "<!-- Page 0 - 0 images -->\nsome line";
        let pages = parse_pages(input).unwrap();
        assert_eq!(pages[0].image_count, 0);
    }

    // #7 Unicode / Devanagari search
    #[test]
    fn test_unicode_search() {
        let input = "<!-- Page 0 - 1 image -->\nnamaḥ śivāya\noṃ bhairavāya namaḥ";
        let pages = parse_pages(input).unwrap();
        let results = search_pages(&pages, "śivāya");
        assert_eq!(results.len(), 1);
        assert!(results[0].line.contains("śivāya"));
    }

    // #8 Last page is just a marker line with no trailing content
    #[test]
    fn test_last_page_marker_only() {
        let input = "<!-- Page 0 - 1 image -->\ncontent\n<!-- Page 1 - 1 image -->";
        let pages = parse_pages(input).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[1].content_line_count(), 0);
    }

    // #9 Page range "0-0" parses correctly
    #[test]
    fn test_page_range_zero_zero() {
        let (start, end) = parse_page_range("0-0").unwrap();
        assert_eq!((start, end), (0, 0));
    }

    // #10 Split to non-writable directory propagates IO error
    #[test]
    fn test_split_non_writable_dir() {
        let input = "<!-- Page 0 - 1 image -->\ncontent";
        let pages = parse_pages(input).unwrap();
        let result = split_pages(&pages, std::path::Path::new("/nonexistent/deeply/nested/dir"));
        assert!(matches!(result, Err(ChunkerError::Io(_))));
    }

    // #11 Marker with extra whitespace
    #[test]
    fn test_marker_extra_whitespace() {
        let input = "<!--  Page  0  -  2  images  -->\nsome content";
        let pages = parse_pages(input).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].number, 0);
        assert_eq!(pages[0].image_count, 2);
    }

    // #12 Multi-digit image counts
    #[test]
    fn test_multi_digit_image_count() {
        let input = "<!-- Page 0 - 123 images -->\nsome content";
        let pages = parse_pages(input).unwrap();
        assert_eq!(pages[0].image_count, 123);
    }

    // -- raw_lines / raw_lines_sans_marker ------------------------------------

    #[test]
    fn test_raw_lines_include_marker_and_blanks() {
        let pages = parse_pages(SAMPLE).unwrap();
        let raw = pages[0].raw_lines();
        // Page 0 has: marker, "First page line one", "First page line two", blank line
        assert_eq!(raw[0], "<!-- Page 0 - 2 images -->");
        assert_eq!(raw[1], "First page line one");
        assert_eq!(raw[2], "First page line two");
        assert_eq!(raw[3], "");
        assert_eq!(raw.len(), 4);
    }

    #[test]
    fn test_no_markers_lines_skip_marker_keep_blanks() {
        let pages = parse_pages(SAMPLE).unwrap();
        let sans = pages[0].raw_lines_sans_marker();
        // Same as raw but without the marker at index 0
        assert_eq!(sans.len(), 3);
        assert_eq!(sans[0], "First page line one");
        assert_eq!(sans[1], "First page line two");
        assert_eq!(sans[2], "");
    }

    // -- collect_page_lines with LineMode -------------------------------------

    #[test]
    fn test_collect_raw_single_page() {
        let pages = parse_pages(SAMPLE).unwrap();
        let lines = collect_page_lines(&pages, 0, 0, LineMode::Raw).unwrap();
        assert_eq!(lines[0], "<!-- Page 0 - 2 images -->");
        assert_eq!(lines[1], "First page line one");
        assert_eq!(lines[2], "First page line two");
        assert_eq!(lines[3], "");
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn test_collect_no_markers_single_page() {
        let pages = parse_pages(SAMPLE).unwrap();
        let lines = collect_page_lines(&pages, 0, 0, LineMode::NoMarkers).unwrap();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "First page line one");
        assert_eq!(lines[1], "First page line two");
        assert_eq!(lines[2], "");
    }

    #[test]
    fn test_raw_lines_range() {
        let pages = parse_pages(SAMPLE).unwrap();
        let lines = collect_page_lines(&pages, 0, 1, LineMode::Raw).unwrap();
        // Page 0: marker + 2 content + blank = 4 lines
        // Page 1: marker + content = 2 lines
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0], "<!-- Page 0 - 2 images -->");
        assert_eq!(lines[4], "<!-- Page 1 - 3 images -->");
        assert_eq!(lines[5], "Second page content");
    }

    #[test]
    fn test_no_markers_range() {
        let pages = parse_pages(SAMPLE).unwrap();
        let lines = collect_page_lines(&pages, 0, 1, LineMode::NoMarkers).unwrap();
        // Page 0 sans marker: 3 lines; Page 1 sans marker: 1 line
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "First page line one");
        assert_eq!(lines[3], "Second page content");
    }

    #[test]
    fn test_collect_content_mode_unchanged() {
        // Default Content mode should behave identically to the original
        let pages = parse_pages(SAMPLE).unwrap();
        let lines = collect_page_lines(&pages, 0, 0, LineMode::Content).unwrap();
        assert_eq!(lines, vec!["First page line one", "First page line two"]);
    }
}
