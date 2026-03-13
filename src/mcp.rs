use crate::{
    ChunkerError, LineMode, chunk_document, chunk_pages, chunks_to_json,
    collect_page_lines, lines_to_json, pages_to_json, parse_page_range,
    parse_pages, search_pages, search_to_json, split_pages, validate_extension,
};
use serde::Serialize;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::schemars;
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Handler functions (content-based, no file I/O — testable)
// ---------------------------------------------------------------------------

pub fn handle_pages_content(content: &str) -> Result<String, ChunkerError> {
    let pages = parse_pages(content)?;
    Ok(pages_to_json(&pages))
}

pub fn handle_lines_content(
    content: &str,
    page: &str,
    mode: Option<&str>,
) -> Result<String, ChunkerError> {
    let pages = parse_pages(content)?;
    let (start, end) = parse_page_range(page)?;
    let line_mode = match mode {
        None | Some("content") => LineMode::Content,
        Some("raw") => LineMode::Raw,
        Some("no_markers") => LineMode::NoMarkers,
        Some(other) => return Err(ChunkerError::InvalidLineMode(other.to_string())),
    };
    let result = collect_page_lines(&pages, start, end, line_mode)?;
    Ok(lines_to_json(
        &result.lines,
        start,
        end,
        result.line_start,
        result.line_end,
    ))
}

pub fn handle_search_content(content: &str, term: &str) -> Result<String, ChunkerError> {
    let pages = parse_pages(content)?;
    let matches = search_pages(&pages, term);
    Ok(search_to_json(&matches, term))
}

pub fn handle_split_content(content: &str, outdir: &str) -> Result<String, ChunkerError> {
    let pages = parse_pages(content)?;
    let outdir_path = std::path::Path::new(outdir);
    split_pages(&pages, outdir_path)?;
    let files: Vec<String> = pages
        .iter()
        .map(|p| format!("page-{:03}.md", p.number))
        .collect();
    #[derive(Serialize)]
    struct SplitJson {
        pages_written: usize,
        outdir: String,
        files: Vec<String>,
    }
    let result = SplitJson {
        pages_written: pages.len(),
        outdir: outdir.to_string(),
        files,
    };
    Ok(serde_json::to_string_pretty(&result).unwrap())
}

pub fn handle_chunks_content(content: &str, per_page: bool) -> Result<String, ChunkerError> {
    if content.trim().is_empty() {
        return Err(ChunkerError::EmptyFile);
    }
    if per_page {
        let pages = parse_pages(content)?;
        let chunks = chunk_pages(&pages);
        Ok(chunks_to_json(&chunks, "per_page"))
    } else {
        let chunks = chunk_document(content);
        Ok(chunks_to_json(&chunks, "document"))
    }
}

// ---------------------------------------------------------------------------
// File-reading wrappers
// ---------------------------------------------------------------------------

pub fn handle_pages(file: &str) -> Result<String, ChunkerError> {
    validate_extension(file)?;
    let content = std::fs::read_to_string(file)?;
    handle_pages_content(&content)
}

pub fn handle_lines(file: &str, page: &str, mode: Option<&str>) -> Result<String, ChunkerError> {
    validate_extension(file)?;
    let content = std::fs::read_to_string(file)?;
    handle_lines_content(&content, page, mode)
}

pub fn handle_search(file: &str, term: &str) -> Result<String, ChunkerError> {
    validate_extension(file)?;
    let content = std::fs::read_to_string(file)?;
    handle_search_content(&content, term)
}

pub fn handle_split(file: &str, outdir: &str) -> Result<String, ChunkerError> {
    validate_extension(file)?;
    let content = std::fs::read_to_string(file)?;
    handle_split_content(&content, outdir)
}

pub fn handle_chunks(file: &str, per_page: bool) -> Result<String, ChunkerError> {
    validate_extension(file)?;
    let content = std::fs::read_to_string(file)?;
    handle_chunks_content(&content, per_page)
}

// ---------------------------------------------------------------------------
// MCP Server
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PagesInput {
    /// Path to a .md or .txt file containing page markers
    pub file: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LinesInput {
    /// Path to a .md or .txt file containing page markers
    pub file: String,
    /// Page number or range (e.g. "5" or "5-10")
    pub page: String,
    /// Line output mode: "content" (default, non-empty lines only), "raw" (all lines including marker), "no_markers" (all lines except marker)
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchInput {
    /// Path to a .md or .txt file containing page markers
    pub file: String,
    /// Search term (case-insensitive substring match)
    pub term: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SplitInput {
    /// Path to a .md or .txt file containing page markers
    pub file: String,
    /// Output directory to write individual page files into (created if it doesn't exist)
    pub outdir: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ChunksInput {
    /// Path to a .md or .txt file
    pub file: String,
    /// If true, chunk per page (requires page markers). If false/omitted, chunk the whole document.
    pub per_page: Option<bool>,
}

#[derive(Clone)]
pub struct TextChunkerMcp {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl TextChunkerMcp {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "List all pages in a page-marked document with page numbers, image counts, line ranges, and content line counts. Returns JSON.")]
    fn pages(
        &self,
        Parameters(input): Parameters<PagesInput>,
    ) -> Result<CallToolResult, McpError> {
        match handle_pages(&input.file) {
            Ok(json) => Ok(CallToolResult::success(vec![Content::text(json)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    #[tool(description = "Get lines from a specific page or page range in a page-marked document. Returns JSON with the lines and source line numbers.")]
    fn lines(
        &self,
        Parameters(input): Parameters<LinesInput>,
    ) -> Result<CallToolResult, McpError> {
        match handle_lines(&input.file, &input.page, input.mode.as_deref()) {
            Ok(json) => Ok(CallToolResult::success(vec![Content::text(json)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    #[tool(description = "Search for a term across all pages in a page-marked document. Case-insensitive substring match. Returns JSON with matching lines and their page numbers.")]
    fn search(
        &self,
        Parameters(input): Parameters<SearchInput>,
    ) -> Result<CallToolResult, McpError> {
        match handle_search(&input.file, &input.term) {
            Ok(json) => Ok(CallToolResult::success(vec![Content::text(json)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    #[tool(description = "Chunk a markdown document into structural segments (headings, paragraphs, list items, code blocks, tables, blockquotes, definition items) for embedding. Supports GFM extensions, math, footnotes, task lists, wikilinks, and Obsidian syntax. Returns JSON with chunk text, type, heading context, and source line numbers. Use per_page=true to chunk within each page separately.")]
    fn chunks(
        &self,
        Parameters(input): Parameters<ChunksInput>,
    ) -> Result<CallToolResult, McpError> {
        match handle_chunks(&input.file, input.per_page.unwrap_or(false)) {
            Ok(json) => Ok(CallToolResult::success(vec![Content::text(json)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    #[tool(description = "Split a page-marked document into individual page files. Each page is written as page-000.md, page-001.md, etc. Returns JSON with pages_written count and file list.")]
    fn split(
        &self,
        Parameters(input): Parameters<SplitInput>,
    ) -> Result<CallToolResult, McpError> {
        match handle_split(&input.file, &input.outdir) {
            Ok(json) => Ok(CallToolResult::success(vec![Content::text(json)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }
}

#[tool_handler]
impl ServerHandler for TextChunkerMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "text-chunker",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "text-chunker: split, query, and chunk page-marked manuscripts (<!-- Page N - M images -->). \
                 Use 'pages' to list all pages, 'lines' to read specific pages, 'search' to find text, \
                 'split' to write individual page files, 'chunks' to extract structural segments for embedding.",
            )
    }
}

pub async fn run_mcp_server() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter("info")
        .init();
    let server = TextChunkerMcp::new()
        .serve(rmcp::transport::stdio())
        .await?;
    server.waiting().await?;
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

    // -- handle_pages_content ------------------------------------------------

    #[test]
    fn test_pages_content_valid_json() {
        let json = handle_pages_content(SAMPLE).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["total_pages"], 3);
        assert_eq!(v["pages"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_pages_content_empty_file() {
        let result = handle_pages_content("");
        assert!(matches!(result, Err(ChunkerError::EmptyFile)));
    }

    #[test]
    fn test_pages_content_no_markers() {
        let result = handle_pages_content("just some text\nno markers here");
        assert!(matches!(result, Err(ChunkerError::NoPagesFound)));
    }

    // -- handle_lines_content ------------------------------------------------

    #[test]
    fn test_lines_content_single_page() {
        let json = handle_lines_content(SAMPLE, "0", None).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["page_start"], 0);
        assert_eq!(v["page_end"], 0);
        assert_eq!(v["count"], 2);
    }

    #[test]
    fn test_lines_content_range() {
        let json = handle_lines_content(SAMPLE, "0-1", None).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["page_start"], 0);
        assert_eq!(v["page_end"], 1);
        assert_eq!(v["count"], 3);
    }

    #[test]
    fn test_lines_content_raw_mode() {
        let json = handle_lines_content(SAMPLE, "0", Some("raw")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let lines = v["lines"].as_array().unwrap();
        assert_eq!(lines[0].as_str().unwrap(), "<!-- Page 0 - 2 images -->");
        assert_eq!(v["count"], 4);
    }

    #[test]
    fn test_lines_content_no_markers_mode() {
        let json = handle_lines_content(SAMPLE, "0", Some("no_markers")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let lines = v["lines"].as_array().unwrap();
        // Should not include the marker line
        assert!(!lines
            .iter()
            .any(|l| l.as_str().unwrap().contains("<!-- Page")));
        assert_eq!(v["count"], 3);
    }

    #[test]
    fn test_lines_content_default_is_content_mode() {
        let json_default = handle_lines_content(SAMPLE, "0", None).unwrap();
        let json_explicit = handle_lines_content(SAMPLE, "0", Some("content")).unwrap();
        assert_eq!(json_default, json_explicit);
    }

    #[test]
    fn test_lines_content_invalid_mode() {
        let result = handle_lines_content(SAMPLE, "0", Some("bogus"));
        assert!(matches!(result, Err(ChunkerError::InvalidLineMode(_))));
    }

    #[test]
    fn test_lines_content_page_not_found() {
        let result = handle_lines_content(SAMPLE, "99", None);
        assert!(matches!(result, Err(ChunkerError::PageNotFound(99))));
    }

    #[test]
    fn test_lines_content_invalid_range() {
        let result = handle_lines_content(SAMPLE, "abc", None);
        assert!(matches!(result, Err(ChunkerError::InvalidPageRange(_))));
    }

    #[test]
    fn test_lines_content_has_line_start_end() {
        let json = handle_lines_content(SAMPLE, "0", None).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["line_start"], 2);
        assert_eq!(v["line_end"], 3);
    }

    // -- handle_search_content -----------------------------------------------

    #[test]
    fn test_search_content_finds_results() {
        let json = handle_search_content(SAMPLE, "page line").unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["total_matches"], 2);
        assert_eq!(v["term"], "page line");
    }

    #[test]
    fn test_search_content_case_insensitive() {
        let json = handle_search_content(SAMPLE, "FIRST").unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["total_matches"], 2);
    }

    #[test]
    fn test_search_content_no_results() {
        let json = handle_search_content(SAMPLE, "zzzznotfound").unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["total_matches"], 0);
    }

    #[test]
    fn test_search_content_empty_file() {
        let result = handle_search_content("", "anything");
        assert!(matches!(result, Err(ChunkerError::EmptyFile)));
    }

    // -- handle_split_content ------------------------------------------------

    #[test]
    fn test_split_content_valid_json() {
        let dir = tempfile::tempdir().unwrap();
        let json = handle_split_content(SAMPLE, dir.path().to_str().unwrap()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["pages_written"], 3);
        assert!(v["outdir"].as_str().unwrap().len() > 0);
    }

    #[test]
    fn test_split_content_creates_files() {
        let dir = tempfile::tempdir().unwrap();
        handle_split_content(SAMPLE, dir.path().to_str().unwrap()).unwrap();
        assert!(dir.path().join("page-000.md").exists());
        assert!(dir.path().join("page-001.md").exists());
        assert!(dir.path().join("page-002.md").exists());
    }

    #[test]
    fn test_split_content_returns_files_list() {
        let dir = tempfile::tempdir().unwrap();
        let json = handle_split_content(SAMPLE, dir.path().to_str().unwrap()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let files = v["files"].as_array().unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].as_str().unwrap(), "page-000.md");
        assert_eq!(files[1].as_str().unwrap(), "page-001.md");
        assert_eq!(files[2].as_str().unwrap(), "page-002.md");
    }

    #[test]
    fn test_split_content_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let result = handle_split_content("", dir.path().to_str().unwrap());
        assert!(matches!(result, Err(ChunkerError::EmptyFile)));
    }

    #[test]
    fn test_split_content_no_markers() {
        let dir = tempfile::tempdir().unwrap();
        let result = handle_split_content("just text", dir.path().to_str().unwrap());
        assert!(matches!(result, Err(ChunkerError::NoPagesFound)));
    }

    // -- handle_chunks_content -----------------------------------------------

    #[test]
    fn test_handle_chunks_content_document() {
        let json = handle_chunks_content("# Hello\n\nWorld", false).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["mode"], "document");
        assert_eq!(v["total_chunks"], 2);
        assert!(v["chunks"].as_array().unwrap().len() == 2);
    }

    #[test]
    fn test_handle_chunks_content_per_page() {
        let json = handle_chunks_content(SAMPLE, true).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["mode"], "per_page");
        assert!(v["total_chunks"].as_u64().unwrap() > 0);
        // All chunks should have page_number
        for chunk in v["chunks"].as_array().unwrap() {
            assert!(chunk["page_number"].is_number());
        }
    }

    #[test]
    fn test_handle_chunks_content_empty() {
        let result = handle_chunks_content("", false);
        assert!(matches!(result, Err(ChunkerError::EmptyFile)));
    }

    #[test]
    fn test_handle_chunks_content_no_markers_per_page() {
        let result = handle_chunks_content("just plain markdown\n\nno markers", true);
        assert!(matches!(result, Err(ChunkerError::NoPagesFound)));
    }
}
