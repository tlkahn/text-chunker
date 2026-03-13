use pulldown_cmark::{Event, MetadataBlockKind, Options, Parser, Tag, TagEnd};
use regex::Regex;
use serde::Serialize;
use thiserror::Error;

pub mod mcp;

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
    #[error("invalid line mode: {0}")]
    InvalidLineMode(String),
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

pub struct LinesResult {
    pub lines: Vec<String>,
    pub line_start: usize, // 1-based, 0 if empty
    pub line_end: usize,   // 1-based, 0 if empty
}

pub fn collect_page_lines(
    pages: &[Page],
    start: usize,
    end: usize,
    mode: LineMode,
) -> Result<LinesResult, ChunkerError> {
    let mut lines = Vec::new();
    let mut line_start: usize = 0;
    let mut line_end: usize = 0;

    for num in start..=end {
        let page = pages
            .iter()
            .find(|p| p.number == num)
            .ok_or(ChunkerError::PageNotFound(num))?;
        match mode {
            LineMode::Content => {
                for (i, raw_line) in page.lines.iter().enumerate().skip(1) {
                    if !raw_line.trim().is_empty() {
                        let src_line = page.start_line + i;
                        if line_start == 0 {
                            line_start = src_line;
                        }
                        line_end = src_line;
                        lines.push(raw_line.to_string());
                    }
                }
            }
            LineMode::Raw => {
                lines.extend_from_slice(page.raw_lines());
                if line_start == 0 {
                    line_start = page.start_line;
                }
                line_end = page.end_line;
            }
            LineMode::NoMarkers => {
                lines.extend_from_slice(page.raw_lines_sans_marker());
                if line_start == 0 && !page.lines.is_empty() {
                    line_start = page.start_line + 1;
                }
                line_end = page.end_line;
            }
        }
    }
    Ok(LinesResult { lines, line_start, line_end })
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

pub fn lines_to_json(lines: &[String], start: usize, end: usize, line_start: usize, line_end: usize) -> String {
    #[derive(Serialize)]
    struct LinesJson {
        page_start: usize,
        page_end: usize,
        line_start: usize,
        line_end: usize,
        count: usize,
        lines: Vec<String>,
    }
    let output = LinesJson {
        page_start: start,
        page_end: end,
        line_start,
        line_end,
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
// Chunk types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkType {
    Heading,
    Paragraph,
    ListItem,
    CodeBlock,
    Table,
    BlockQuote,
    DefinitionItem,
}

#[derive(Debug, Clone, Serialize)]
pub struct Chunk {
    pub text: String,
    pub chunk_type: ChunkType,
    pub heading_context: Vec<String>,
    pub heading_level: Option<u8>,
    pub page_number: Option<usize>,
    pub source_line_start: usize,
    pub source_line_end: usize,
}

// ---------------------------------------------------------------------------
// Line index helpers
// ---------------------------------------------------------------------------

pub fn build_line_index(content: &str) -> Vec<usize> {
    let mut index = vec![0];
    for (i, b) in content.bytes().enumerate() {
        if b == b'\n' && i + 1 <= content.len() {
            index.push(i + 1);
        }
    }
    index
}

pub fn byte_offset_to_line(index: &[usize], offset: usize) -> usize {
    match index.binary_search(&offset) {
        Ok(pos) => pos + 1,
        Err(pos) => pos,
    }
}

// ---------------------------------------------------------------------------
// Obsidian pre-processing
// ---------------------------------------------------------------------------

pub fn preprocess_obsidian(content: &str) -> String {
    let re_comment = Regex::new(r"%%[\s\S]*?%%").unwrap();
    let result = re_comment.replace_all(content, "");
    let re_highlight = Regex::new(r"==([^=]+)==").unwrap();
    let result = re_highlight.replace_all(&result, "$1");
    // Strip #^block-ref from wikilinks: [[Page#^foo]] → [[Page]], [[Page#^foo|text]] unchanged
    let re_block_ref = Regex::new(r"\[\[([^\]|]+?)#\^[a-zA-Z0-9_-]+\]\]").unwrap();
    let result = re_block_ref.replace_all(&result, "[[$1]]");
    // Strip ^block-id anchors at end of line (space + ^identifier)
    let re_anchor = Regex::new(r"(?m) \^[a-zA-Z0-9_-]+$").unwrap();
    re_anchor.replace_all(&result, "").into_owned()
}

// ---------------------------------------------------------------------------
// Core chunking algorithm
// ---------------------------------------------------------------------------

pub fn chunk_markdown(content: &str) -> Vec<Chunk> {
    if content.trim().is_empty() {
        return Vec::new();
    }

    let content = preprocess_obsidian(content);
    let line_index = build_line_index(&content);

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_MATH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_SMART_PUNCTUATION);
    opts.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    opts.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    opts.insert(Options::ENABLE_GFM);
    opts.insert(Options::ENABLE_DEFINITION_LIST);
    opts.insert(Options::ENABLE_WIKILINKS);

    let parser = Parser::new_ext(&content, opts).into_offset_iter();

    let mut chunks: Vec<Chunk> = Vec::new();
    let mut heading_stack: Vec<(u8, String)> = Vec::new();

    // State for current block being accumulated
    let mut text_buf = String::new();
    let mut block_type: Option<ChunkType> = None;
    let mut block_start_offset: usize = 0;
    #[allow(unused_assignments)]
    let mut block_end_offset: usize = 0;
    let mut current_heading_level: Option<u8> = None;

    // Nesting depth for list items, blockquotes, footnotes, and definitions
    let mut item_depth: usize = 0;
    let mut blockquote_depth: usize = 0;
    let mut _list_depth: usize = 0;
    let mut footnote_depth: usize = 0;
    let mut in_frontmatter = false;
    let mut def_title_buf = String::new();
    let mut def_depth: usize = 0;
    let mut def_start_offset: usize = 0;

    for (event, range) in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                block_type = Some(ChunkType::Heading);
                current_heading_level = Some(level as u8);
                text_buf.clear();
                block_start_offset = range.start;
            }
            Event::End(TagEnd::Heading(_)) => {
                block_end_offset = range.end;
                let text = text_buf.trim().to_string();
                if !text.is_empty() {
                    let level = current_heading_level.unwrap();
                    // Update heading stack: pop levels >= current
                    heading_stack.retain(|(l, _)| *l < level);
                    heading_stack.push((level, text.clone()));

                    chunks.push(Chunk {
                        text,
                        chunk_type: ChunkType::Heading,
                        heading_context: heading_stack.iter().map(|(_, s)| s.clone()).collect(),
                        heading_level: current_heading_level,
                        page_number: None,
                        source_line_start: byte_offset_to_line(&line_index, block_start_offset),
                        source_line_end: byte_offset_to_line(&line_index, block_end_offset.saturating_sub(1).max(block_start_offset)),
                    });
                }
                block_type = None;
                current_heading_level = None;
                text_buf.clear();
            }
            Event::Start(Tag::Paragraph) => {
                if item_depth == 0 && blockquote_depth == 0 && footnote_depth == 0 && def_depth == 0 {
                    block_type = Some(ChunkType::Paragraph);
                    text_buf.clear();
                    block_start_offset = range.start;
                }
                // Inside items/blockquotes/footnotes/definitions, paragraph text just accumulates
            }
            Event::End(TagEnd::Paragraph) => {
                if item_depth == 0 && blockquote_depth == 0 && footnote_depth == 0 && def_depth == 0 {
                    block_end_offset = range.end;
                    let text = text_buf.trim().to_string();
                    if !text.is_empty() {
                        chunks.push(Chunk {
                            text,
                            chunk_type: ChunkType::Paragraph,
                            heading_context: heading_stack.iter().map(|(_, s)| s.clone()).collect(),
                            heading_level: None,
                            page_number: None,
                            source_line_start: byte_offset_to_line(&line_index, block_start_offset),
                            source_line_end: byte_offset_to_line(&line_index, block_end_offset.saturating_sub(1).max(block_start_offset)),
                        });
                    }
                    block_type = None;
                    text_buf.clear();
                }
            }
            Event::Start(Tag::List(_)) => {
                _list_depth += 1;
            }
            Event::End(TagEnd::List(_)) => {
                _list_depth = _list_depth.saturating_sub(1);
            }
            Event::Start(Tag::Item) => {
                item_depth += 1;
                if item_depth == 1 {
                    text_buf.clear();
                    block_start_offset = range.start;
                }
            }
            Event::End(TagEnd::Item) => {
                if item_depth == 1 {
                    block_end_offset = range.end;
                    let text = text_buf.trim().to_string();
                    if !text.is_empty() {
                        chunks.push(Chunk {
                            text,
                            chunk_type: ChunkType::ListItem,
                            heading_context: heading_stack.iter().map(|(_, s)| s.clone()).collect(),
                            heading_level: None,
                            page_number: None,
                            source_line_start: byte_offset_to_line(&line_index, block_start_offset),
                            source_line_end: byte_offset_to_line(&line_index, block_end_offset.saturating_sub(1).max(block_start_offset)),
                        });
                    }
                    text_buf.clear();
                }
                item_depth = item_depth.saturating_sub(1);
            }
            Event::Start(Tag::CodeBlock(_)) => {
                block_type = Some(ChunkType::CodeBlock);
                text_buf.clear();
                block_start_offset = range.start;
            }
            Event::End(TagEnd::CodeBlock) => {
                block_end_offset = range.end;
                let text = text_buf.trim().to_string();
                if !text.is_empty() {
                    chunks.push(Chunk {
                        text,
                        chunk_type: ChunkType::CodeBlock,
                        heading_context: heading_stack.iter().map(|(_, s)| s.clone()).collect(),
                        heading_level: None,
                        page_number: None,
                        source_line_start: byte_offset_to_line(&line_index, block_start_offset),
                        source_line_end: byte_offset_to_line(&line_index, block_end_offset.saturating_sub(1).max(block_start_offset)),
                    });
                }
                block_type = None;
                text_buf.clear();
            }
            Event::Start(Tag::Table(_)) => {
                block_type = Some(ChunkType::Table);
                text_buf.clear();
                block_start_offset = range.start;
            }
            Event::End(TagEnd::Table) => {
                block_end_offset = range.end;
                let text = text_buf.trim().to_string();
                if !text.is_empty() {
                    chunks.push(Chunk {
                        text,
                        chunk_type: ChunkType::Table,
                        heading_context: heading_stack.iter().map(|(_, s)| s.clone()).collect(),
                        heading_level: None,
                        page_number: None,
                        source_line_start: byte_offset_to_line(&line_index, block_start_offset),
                        source_line_end: byte_offset_to_line(&line_index, block_end_offset.saturating_sub(1).max(block_start_offset)),
                    });
                }
                block_type = None;
                text_buf.clear();
            }
            Event::Start(Tag::BlockQuote(_)) => {
                blockquote_depth += 1;
                if blockquote_depth == 1 {
                    block_type = Some(ChunkType::BlockQuote);
                    text_buf.clear();
                    block_start_offset = range.start;
                }
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                if blockquote_depth == 1 {
                    block_end_offset = range.end;
                    let text = text_buf.trim().to_string();
                    if !text.is_empty() {
                        chunks.push(Chunk {
                            text,
                            chunk_type: ChunkType::BlockQuote,
                            heading_context: heading_stack.iter().map(|(_, s)| s.clone()).collect(),
                            heading_level: None,
                            page_number: None,
                            source_line_start: byte_offset_to_line(&line_index, block_start_offset),
                            source_line_end: byte_offset_to_line(&line_index, block_end_offset.saturating_sub(1).max(block_start_offset)),
                        });
                    }
                    block_type = None;
                    text_buf.clear();
                }
                blockquote_depth = blockquote_depth.saturating_sub(1);
            }
            // Footnote handling
            Event::Start(Tag::FootnoteDefinition(_)) => {
                footnote_depth += 1;
                if footnote_depth == 1 {
                    text_buf.clear();
                    block_start_offset = range.start;
                }
            }
            Event::End(TagEnd::FootnoteDefinition) => {
                if footnote_depth == 1 {
                    block_end_offset = range.end;
                    let text = text_buf.trim().to_string();
                    if !text.is_empty() {
                        chunks.push(Chunk {
                            text,
                            chunk_type: ChunkType::Paragraph,
                            heading_context: heading_stack.iter().map(|(_, s)| s.clone()).collect(),
                            heading_level: None,
                            page_number: None,
                            source_line_start: byte_offset_to_line(&line_index, block_start_offset),
                            source_line_end: byte_offset_to_line(&line_index, block_end_offset.saturating_sub(1).max(block_start_offset)),
                        });
                    }
                    block_type = None;
                    text_buf.clear();
                }
                footnote_depth = footnote_depth.saturating_sub(1);
            }
            Event::FootnoteReference(label) => {
                text_buf.push_str("[^");
                text_buf.push_str(&label);
                text_buf.push(']');
            }
            // Math handling
            Event::InlineMath(text) => {
                text_buf.push_str(&text);
            }
            Event::DisplayMath(text) => {
                text_buf.push_str(&text);
            }
            // Task list handling
            Event::TaskListMarker(checked) => {
                text_buf.push_str(if checked { "[x] " } else { "[ ] " });
            }
            // Frontmatter handling — drop silently
            Event::Start(Tag::MetadataBlock(MetadataBlockKind::YamlStyle | MetadataBlockKind::PlusesStyle)) => {
                in_frontmatter = true;
            }
            Event::End(TagEnd::MetadataBlock(_)) => {
                in_frontmatter = false;
                text_buf.clear();
            }
            // Definition list handling
            Event::Start(Tag::DefinitionList) | Event::End(TagEnd::DefinitionList) => {}
            Event::Start(Tag::DefinitionListTitle) => {
                def_depth += 1;
                def_title_buf.clear();
                text_buf.clear();
                def_start_offset = range.start;
            }
            Event::End(TagEnd::DefinitionListTitle) => {
                def_title_buf = text_buf.trim().to_string();
                text_buf.clear();
                def_depth = def_depth.saturating_sub(1);
            }
            Event::Start(Tag::DefinitionListDefinition) => {
                def_depth += 1;
                text_buf.clear();
            }
            Event::End(TagEnd::DefinitionListDefinition) => {
                block_end_offset = range.end;
                let def_text = text_buf.trim().to_string();
                if !def_title_buf.is_empty() || !def_text.is_empty() {
                    let combined = format!("{}: {}", def_title_buf, def_text);
                    chunks.push(Chunk {
                        text: combined,
                        chunk_type: ChunkType::DefinitionItem,
                        heading_context: heading_stack.iter().map(|(_, s)| s.clone()).collect(),
                        heading_level: None,
                        page_number: None,
                        source_line_start: byte_offset_to_line(&line_index, def_start_offset),
                        source_line_end: byte_offset_to_line(&line_index, block_end_offset.saturating_sub(1).max(def_start_offset)),
                    });
                }
                text_buf.clear();
                def_depth = def_depth.saturating_sub(1);
            }
            // Table cell separators
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                if block_type == Some(ChunkType::Table) {
                    text_buf.push('\n');
                }
            }
            Event::End(TagEnd::TableCell) => {
                if block_type == Some(ChunkType::Table) {
                    text_buf.push('\t');
                }
            }
            // Text content events
            Event::Text(text) => {
                if !in_frontmatter {
                    text_buf.push_str(&text);
                }
            }
            Event::Code(code) => {
                text_buf.push_str(&code);
            }
            Event::SoftBreak => {
                text_buf.push(' ');
            }
            Event::HardBreak => {
                text_buf.push('\n');
            }
            _ => {}
        }
    }

    chunks
}

// ---------------------------------------------------------------------------
// Document + page wrappers
// ---------------------------------------------------------------------------

pub fn chunk_document(content: &str) -> Vec<Chunk> {
    chunk_markdown(content)
}

pub fn chunk_pages(pages: &[Page]) -> Vec<Chunk> {
    let mut all_chunks = Vec::new();
    for page in pages {
        let page_content = page.raw_lines_sans_marker().join("\n");
        let mut chunks = chunk_markdown(&page_content);
        for chunk in &mut chunks {
            chunk.page_number = Some(page.number);
            // Adjust line numbers: add page start_line offset (start_line is 1-based for the marker,
            // content starts at start_line + 1, so offset is start_line)
            chunk.source_line_start += page.start_line;
            chunk.source_line_end += page.start_line;
        }
        all_chunks.extend(chunks);
    }
    all_chunks
}

// ---------------------------------------------------------------------------
// Chunk output formatting
// ---------------------------------------------------------------------------

pub fn chunks_to_json(chunks: &[Chunk], mode: &str) -> String {
    #[derive(Serialize)]
    struct ChunksJson<'a> {
        total_chunks: usize,
        mode: &'a str,
        chunks: &'a [Chunk],
    }
    let output = ChunksJson {
        total_chunks: chunks.len(),
        mode,
        chunks,
    };
    serde_json::to_string_pretty(&output).unwrap()
}

pub fn format_chunks_table(chunks: &[Chunk]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{:<6} {:<12} {:<14} {}\n",
        "#", "Type", "Lines", "Text"
    ));
    out.push_str(&format!("{}\n", "-".repeat(72)));
    for (i, chunk) in chunks.iter().enumerate() {
        let type_str = match chunk.chunk_type {
            ChunkType::Heading => "heading",
            ChunkType::Paragraph => "paragraph",
            ChunkType::ListItem => "list_item",
            ChunkType::CodeBlock => "code_block",
            ChunkType::Table => "table",
            ChunkType::BlockQuote => "block_quote",
            ChunkType::DefinitionItem => "def_item",
        };
        let preview: String = chunk.text.chars().take(40).collect();
        let preview = preview.replace('\n', " ");
        let suffix = if chunk.text.len() > 40 { "..." } else { "" };
        out.push_str(&format!(
            "{:<6} {:<12} {:<14} {}{}\n",
            i + 1,
            type_str,
            format!("{}-{}", chunk.source_line_start, chunk.source_line_end),
            preview,
            suffix
        ));
    }
    out.push_str(&format!("{}\n", "-".repeat(72)));
    out.push_str(&format!("Total: {} chunks\n", chunks.len()));
    out
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
        let result = collect_page_lines(&pages, 0, 0, LineMode::Content).unwrap();
        assert_eq!(result.lines, vec!["First page line one", "First page line two"]);
    }

    #[test]
    fn test_collect_page_range_lines() {
        let pages = parse_pages(SAMPLE).unwrap();
        let result = collect_page_lines(&pages, 0, 1, LineMode::Content).unwrap();
        assert_eq!(
            result.lines,
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
        let result = collect_page_lines(&pages, 0, 0, LineMode::Content).unwrap();
        let json_str = lines_to_json(&result.lines, 0, 0, result.line_start, result.line_end);
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
        let result = collect_page_lines(&pages, 3, 3, LineMode::Content).unwrap();
        assert_eq!(result.lines, vec!["first version"]);
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

    // -- build_line_index / byte_offset_to_line --------------------------------

    #[test]
    fn test_build_line_index_basic() {
        let index = build_line_index("abc\ndef\nghi");
        assert_eq!(index, vec![0, 4, 8]);
    }

    #[test]
    fn test_byte_offset_to_line() {
        let index = build_line_index("abc\ndef\nghi");
        assert_eq!(byte_offset_to_line(&index, 0), 1); // 'a'
        assert_eq!(byte_offset_to_line(&index, 4), 2); // 'd'
        assert_eq!(byte_offset_to_line(&index, 5), 2); // 'e'
    }

    // -- chunk_markdown -------------------------------------------------------

    #[test]
    fn test_chunk_single_paragraph() {
        let chunks = chunk_markdown("Hello world");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, ChunkType::Paragraph);
        assert_eq!(chunks[0].text, "Hello world");
    }

    #[test]
    fn test_chunk_multiple_paragraphs() {
        let chunks = chunk_markdown("First paragraph\n\nSecond paragraph");
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].text, "First paragraph");
        assert_eq!(chunks[1].text, "Second paragraph");
    }

    #[test]
    fn test_chunk_heading() {
        let chunks = chunk_markdown("# Title");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, ChunkType::Heading);
        assert_eq!(chunks[0].heading_level, Some(1));
        assert_eq!(chunks[0].text, "Title");
    }

    #[test]
    fn test_chunk_heading_context() {
        let chunks = chunk_markdown("# Title\n\nSome text");
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[1].chunk_type, ChunkType::Paragraph);
        assert_eq!(chunks[1].heading_context, vec!["Title"]);
    }

    #[test]
    fn test_chunk_heading_level_reset() {
        let input = "# H1\n\n## H2\n\ntext under h2\n\n# New\n\ntext under new";
        let chunks = chunk_markdown(input);
        // Find the paragraph "text under new"
        let last_para = chunks.iter().find(|c| c.text == "text under new").unwrap();
        assert_eq!(last_para.heading_context, vec!["New"]);
    }

    #[test]
    fn test_chunk_list_items() {
        let chunks = chunk_markdown("- apple\n- banana");
        let items: Vec<&Chunk> = chunks.iter().filter(|c| c.chunk_type == ChunkType::ListItem).collect();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].text, "apple");
        assert_eq!(items[1].text, "banana");
    }

    #[test]
    fn test_chunk_code_block() {
        let chunks = chunk_markdown("```\nlet x = 1;\n```");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, ChunkType::CodeBlock);
        assert_eq!(chunks[0].text, "let x = 1;");
    }

    #[test]
    fn test_chunk_blockquote() {
        let chunks = chunk_markdown("> quoted text");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, ChunkType::BlockQuote);
        assert_eq!(chunks[0].text, "quoted text");
    }

    #[test]
    fn test_chunk_table() {
        let input = "| A | B |\n|---|---|\n| 1 | 2 |";
        let chunks = chunk_markdown(input);
        let tables: Vec<&Chunk> = chunks.iter().filter(|c| c.chunk_type == ChunkType::Table).collect();
        assert_eq!(tables.len(), 1);
        assert!(tables[0].text.contains("A"));
        assert!(tables[0].text.contains("B"));
    }

    #[test]
    fn test_chunk_clean_text() {
        let chunks = chunk_markdown("**bold** and *italic*");
        assert_eq!(chunks[0].text, "bold and italic");
    }

    #[test]
    fn test_chunk_link_text() {
        let chunks = chunk_markdown("[text](https://example.com)");
        assert_eq!(chunks[0].text, "text");
    }

    #[test]
    fn test_chunk_source_lines() {
        let input = "# Title\n\nParagraph here";
        let chunks = chunk_markdown(input);
        assert_eq!(chunks[0].source_line_start, 1);
        assert!(chunks[1].source_line_start >= 3);
    }

    #[test]
    fn test_chunk_html_comments_dropped() {
        let input = "First paragraph\n\n<!-- Page 1 - 2 images -->\n\nSecond paragraph";
        let chunks = chunk_markdown(input);
        let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
        assert!(texts.contains(&"First paragraph"));
        assert!(texts.contains(&"Second paragraph"));
        assert!(!texts.iter().any(|t| t.contains("Page 1")));
    }

    #[test]
    fn test_chunk_unicode() {
        let chunks = chunk_markdown("namaḥ śivāya");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "namaḥ śivāya");
    }

    #[test]
    fn test_chunk_empty() {
        let chunks = chunk_markdown("");
        assert!(chunks.is_empty());
        let chunks2 = chunk_markdown("   \n  \n  ");
        assert!(chunks2.is_empty());
    }

    // -- chunk_document / chunk_pages -----------------------------------------

    #[test]
    fn test_chunk_document_basic() {
        let chunks = chunk_document("# Hello\n\nWorld");
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].chunk_type, ChunkType::Heading);
        assert_eq!(chunks[1].chunk_type, ChunkType::Paragraph);
    }

    #[test]
    fn test_chunk_document_with_page_markers() {
        let input = "<!-- Page 1 - 2 images -->\n# Title\n\nContent\n\n<!-- Page 2 - 1 image -->\n\nMore content";
        let chunks = chunk_document(input);
        // Page markers should be silently dropped
        assert!(!chunks.iter().any(|c| c.text.contains("Page 1")));
        assert!(chunks.iter().any(|c| c.text == "Title"));
        assert!(chunks.iter().any(|c| c.text == "Content"));
    }

    #[test]
    fn test_chunk_pages_basic() {
        let pages = parse_pages(SAMPLE).unwrap();
        let chunks = chunk_pages(&pages);
        assert!(!chunks.is_empty());
        // All chunks should have page_number set
        assert!(chunks.iter().all(|c| c.page_number.is_some()));
    }

    #[test]
    fn test_chunk_pages_heading_isolation() {
        let input = "<!-- Page 1 - 1 image -->\n# Chapter 1\n\nContent A\n\n<!-- Page 2 - 1 image -->\nContent B";
        let pages = parse_pages(input).unwrap();
        let chunks = chunk_pages(&pages);
        let content_b = chunks.iter().find(|c| c.text == "Content B").unwrap();
        // Heading from page 1 should NOT bleed into page 2
        assert!(content_b.heading_context.is_empty());
    }

    // -- chunks_to_json / format_chunks_table ---------------------------------

    #[test]
    fn test_chunks_json_valid() {
        let chunks = chunk_document("# Hello\n\nWorld");
        let json_str = chunks_to_json(&chunks, "document");
        let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["total_chunks"], 2);
        assert_eq!(v["mode"], "document");
        assert_eq!(v["chunks"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_format_chunks_table() {
        let chunks = chunk_document("# Hello\n\nWorld");
        let table = format_chunks_table(&chunks);
        assert!(table.contains("Type"));
        assert!(table.contains("heading"));
        assert!(table.contains("paragraph"));
        assert!(table.contains("Total: 2 chunks"));
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
        let result = collect_page_lines(&pages, 0, 0, LineMode::Raw).unwrap();
        assert_eq!(result.lines[0], "<!-- Page 0 - 2 images -->");
        assert_eq!(result.lines[1], "First page line one");
        assert_eq!(result.lines[2], "First page line two");
        assert_eq!(result.lines[3], "");
        assert_eq!(result.lines.len(), 4);
    }

    #[test]
    fn test_collect_no_markers_single_page() {
        let pages = parse_pages(SAMPLE).unwrap();
        let result = collect_page_lines(&pages, 0, 0, LineMode::NoMarkers).unwrap();
        assert_eq!(result.lines.len(), 3);
        assert_eq!(result.lines[0], "First page line one");
        assert_eq!(result.lines[1], "First page line two");
        assert_eq!(result.lines[2], "");
    }

    #[test]
    fn test_raw_lines_range() {
        let pages = parse_pages(SAMPLE).unwrap();
        let result = collect_page_lines(&pages, 0, 1, LineMode::Raw).unwrap();
        // Page 0: marker + 2 content + blank = 4 lines
        // Page 1: marker + content = 2 lines
        assert_eq!(result.lines.len(), 6);
        assert_eq!(result.lines[0], "<!-- Page 0 - 2 images -->");
        assert_eq!(result.lines[4], "<!-- Page 1 - 3 images -->");
        assert_eq!(result.lines[5], "Second page content");
    }

    #[test]
    fn test_no_markers_range() {
        let pages = parse_pages(SAMPLE).unwrap();
        let result = collect_page_lines(&pages, 0, 1, LineMode::NoMarkers).unwrap();
        // Page 0 sans marker: 3 lines; Page 1 sans marker: 1 line
        assert_eq!(result.lines.len(), 4);
        assert_eq!(result.lines[0], "First page line one");
        assert_eq!(result.lines[3], "Second page content");
    }

    #[test]
    fn test_collect_content_mode_unchanged() {
        // Default Content mode should behave identically to the original
        let pages = parse_pages(SAMPLE).unwrap();
        let lines = collect_page_lines(&pages, 0, 0, LineMode::Content).unwrap();
        assert_eq!(lines.lines, vec!["First page line one", "First page line two"]);
    }

    // -- line_start / line_end ------------------------------------------------

    #[test]
    fn test_lines_json_has_line_start_end() {
        let pages = parse_pages(SAMPLE).unwrap();
        let result = collect_page_lines(&pages, 0, 0, LineMode::Content).unwrap();
        let json_str = lines_to_json(&result.lines, 0, 0, result.line_start, result.line_end);
        let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["line_start"], 2);
        assert_eq!(v["line_end"], 3);
    }

    #[test]
    fn test_lines_json_line_range_content_multipage() {
        let pages = parse_pages(SAMPLE).unwrap();
        let result = collect_page_lines(&pages, 0, 1, LineMode::Content).unwrap();
        let json_str = lines_to_json(&result.lines, 0, 1, result.line_start, result.line_end);
        let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["line_start"], 2);
        assert_eq!(v["line_end"], 6);
    }

    #[test]
    fn test_lines_json_line_range_raw() {
        let pages = parse_pages(SAMPLE).unwrap();
        let result = collect_page_lines(&pages, 0, 0, LineMode::Raw).unwrap();
        let json_str = lines_to_json(&result.lines, 0, 0, result.line_start, result.line_end);
        let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["line_start"], 1);
        assert_eq!(v["line_end"], 4);
    }

    #[test]
    fn test_lines_json_line_range_no_markers() {
        let pages = parse_pages(SAMPLE).unwrap();
        let result = collect_page_lines(&pages, 0, 0, LineMode::NoMarkers).unwrap();
        let json_str = lines_to_json(&result.lines, 0, 0, result.line_start, result.line_end);
        let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["line_start"], 2);
        assert_eq!(v["line_end"], 4);
    }

    #[test]
    fn test_collect_page_lines_returns_line_range() {
        let pages = parse_pages(SAMPLE).unwrap();
        let result = collect_page_lines(&pages, 0, 0, LineMode::Content).unwrap();
        assert_eq!(result.line_start, 2);
        assert_eq!(result.line_end, 3);
    }

    #[test]
    fn test_collect_line_range_empty_content() {
        let input = "<!-- Page 0 - 1 image -->\n   \n\n<!-- Page 1 - 2 images -->\ncontent";
        let pages = parse_pages(input).unwrap();
        let result = collect_page_lines(&pages, 0, 0, LineMode::Content).unwrap();
        assert_eq!(result.line_start, 0);
        assert_eq!(result.line_end, 0);
    }

    // -- Phase A: Fix data loss bugs ------------------------------------------

    #[test]
    fn test_chunk_inline_math() {
        let chunks = chunk_markdown("The equation $E = mc^2$ is famous");
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].text.contains("E = mc^2"));
    }

    #[test]
    fn test_chunk_display_math() {
        let chunks = chunk_markdown("$$\n\\sum_{i=1}^{n} i\n$$");
        assert!(!chunks.is_empty());
        assert!(chunks.iter().any(|c| c.text.contains("\\sum")));
    }

    #[test]
    fn test_chunk_footnote_reference() {
        let chunks = chunk_markdown("Main text[^1] here\n\n[^1]: The footnote text");
        let main = chunks.iter().find(|c| c.text.contains("Main text")).unwrap();
        assert!(main.text.contains("[^1]"));
    }

    #[test]
    fn test_chunk_footnote_definition() {
        let chunks = chunk_markdown("Main text[^1] here\n\n[^1]: The footnote text");
        assert!(chunks.iter().any(|c| c.text.contains("The footnote text")));
    }

    // -- Phase B: Verify existing inline handling -----------------------------

    #[test]
    fn test_chunk_strikethrough() {
        let chunks = chunk_markdown("This is ~~deleted~~ text");
        assert_eq!(chunks[0].text, "This is deleted text");
    }

    // -- Phase C: New pulldown-cmark extensions -------------------------------

    #[test]
    fn test_chunk_task_list() {
        let chunks = chunk_markdown("- [ ] unchecked\n- [x] checked");
        let items: Vec<&Chunk> = chunks.iter().filter(|c| c.chunk_type == ChunkType::ListItem).collect();
        assert_eq!(items.len(), 2);
        assert!(items[0].text.starts_with("[ ] "));
        assert!(items[1].text.starts_with("[x] "));
    }

    #[test]
    fn test_chunk_smart_punctuation() {
        let chunks = chunk_markdown("She said \"hello\" -- and left...");
        let text = &chunks[0].text;
        assert!(text.contains('\u{201c}') || text.contains('\u{201d}')); // smart quotes
        assert!(text.contains('\u{2013}')); // en-dash
    }

    #[test]
    fn test_chunk_heading_attributes() {
        let chunks = chunk_markdown("# Title {#my-id .cls}");
        assert_eq!(chunks[0].text, "Title");
    }

    #[test]
    fn test_chunk_gfm_callout() {
        let chunks = chunk_markdown("> [!NOTE]\n> This is a note");
        let bq = chunks.iter().find(|c| c.chunk_type == ChunkType::BlockQuote).unwrap();
        assert!(bq.text.contains("This is a note"));
    }

    #[test]
    fn test_chunk_wikilink_simple() {
        let chunks = chunk_markdown("See [[Some Page]] for details");
        let text = &chunks[0].text;
        assert!(text.contains("Some Page"));
        assert!(!text.contains("[["));
    }

    #[test]
    fn test_chunk_wikilink_piped() {
        let chunks = chunk_markdown("See [[Page|display text]] for details");
        let text = &chunks[0].text;
        assert!(text.contains("display text"));
    }

    // -- Phase D: Definition lists --------------------------------------------

    #[test]
    fn test_chunk_definition_list() {
        let chunks = chunk_markdown("Term 1\n:   Definition 1\n\nTerm 2\n:   Definition 2");
        let defs: Vec<&Chunk> = chunks.iter().filter(|c| c.chunk_type == ChunkType::DefinitionItem).collect();
        assert_eq!(defs.len(), 2);
        assert!(defs[0].text.contains("Term 1"));
        assert!(defs[0].text.contains("Definition 1"));
        assert!(defs[1].text.contains("Term 2"));
        assert!(defs[1].text.contains("Definition 2"));
    }

    #[test]
    fn test_chunk_definition_list_json() {
        let chunks = chunk_markdown("Term\n:   Definition");
        let json_str = chunks_to_json(&chunks, "document");
        let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        let chunk_types: Vec<&str> = v["chunks"].as_array().unwrap()
            .iter()
            .map(|c| c["chunk_type"].as_str().unwrap())
            .collect();
        assert!(chunk_types.contains(&"definition_item"));
    }

    // -- Phase E: Frontmatter -------------------------------------------------

    #[test]
    fn test_chunk_frontmatter_dropped() {
        let chunks = chunk_markdown("---\ntitle: X\ntags: [a, b]\n---\n\n# Heading\n\nContent");
        assert!(!chunks.iter().any(|c| c.text.contains("title:")));
        assert!(chunks.iter().any(|c| c.text == "Heading"));
        assert!(chunks.iter().any(|c| c.text == "Content"));
    }

    // -- Phase F: Obsidian pre-processing -------------------------------------

    #[test]
    fn test_chunk_obsidian_comment_stripped() {
        let chunks = chunk_markdown("Before %%hidden%% after");
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].text.contains("Before"));
        assert!(chunks[0].text.contains("after"));
        assert!(!chunks[0].text.contains("hidden"));
    }

    #[test]
    fn test_chunk_obsidian_comment_multiline() {
        let chunks = chunk_markdown("Before\n\n%%\nmulti\nline\n%%\n\nAfter");
        assert!(!chunks.iter().any(|c| c.text.contains("multi")));
        assert!(chunks.iter().any(|c| c.text.contains("Before")));
        assert!(chunks.iter().any(|c| c.text.contains("After")));
    }

    #[test]
    fn test_chunk_obsidian_highlight_text_kept() {
        let chunks = chunk_markdown("This has ==highlighted text== inside");
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].text.contains("highlighted text"));
        assert!(!chunks[0].text.contains("=="));
    }

    #[test]
    fn test_chunk_obsidian_block_anchor_stripped() {
        let chunks = chunk_markdown("This is a paragraph ^block-id");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "This is a paragraph");
    }

    #[test]
    fn test_chunk_obsidian_block_anchor_multiline() {
        let chunks = chunk_markdown("First line\nSecond line ^my-ref\n\nAnother paragraph");
        assert_eq!(chunks.len(), 2);
        assert!(!chunks[0].text.contains("^my-ref"));
        assert!(chunks[0].text.contains("Second line"));
    }

    #[test]
    fn test_chunk_obsidian_block_ref_in_wikilink() {
        // [[Page#^foo]] → preprocessed to [[Page]] → rendered as "Page"
        let chunks = chunk_markdown("See [[Page#^block-id]] for details");
        let text = &chunks[0].text;
        assert!(text.contains("Page"));
        assert!(!text.contains("^block-id"));
        assert!(!text.contains("#^"));
    }

    #[test]
    fn test_chunk_obsidian_block_ref_piped_wikilink_unchanged() {
        // [[Page#^foo|display]] — piped wikilinks keep display text as-is
        let chunks = chunk_markdown("See [[Page#^foo|display text]] for details");
        let text = &chunks[0].text;
        assert!(text.contains("display text"));
    }
}
