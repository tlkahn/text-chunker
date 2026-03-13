# text-chunker

A Rust CLI tool for splitting page-marked documents into chunks. Built for OCR-processed Sanskrit manuscripts that use HTML comment markers (`<!-- Page N - M images -->`) to delimit pages.

Includes structural markdown chunking (headings, paragraphs, lists, code blocks, tables, blockquotes) for embedding into vector databases, and an MCP (Model Context Protocol) server so LLM agents can query pages, lines, search results, and chunks programmatically over stdio.

## Installation

```bash
cargo install --path .
```

## Page marker format

The tool recognises markers of the form:

```html
<!-- Page 0 - 2 images -->
<!-- Page 1 - 3 images -->
<!-- Page 42 - 1 image -->
```

Whitespace inside the marker is flexible. Both `image` and `images` are accepted. Content before the first marker is silently ignored.

## Usage

```
text-chunker [--json] <COMMAND> <FILE> [ARGS...]
```

`<FILE>` is a `.md` or `.txt` file path, or `-` for stdin.

### Pages summary

```bash
text-chunker pages manuscript.md
```

```
Page     Images   Lines          Content
----------------------------------------------
0        2        1-84           45
1        0        85-140         37
2        0        141-200        42
----------------------------------------------
Total: 3 pages, 124 content lines
```

Pages with no content lines are flagged as `[empty]`.

### Extract lines from a page

```bash
text-chunker lines manuscript.md 5              # single page (content only)
text-chunker lines manuscript.md 5-10           # page range
text-chunker lines manuscript.md 5 --raw        # all original lines including marker
text-chunker lines manuscript.md 5 --no-markers # all lines except marker comment
```

By default, outputs non-empty content lines (marker lines and blank lines are excluded). Use `--raw` for verbatim original lines (marker + blanks + content), or `--no-markers` for all lines except the marker comment (blanks preserved).

### Search across pages

```bash
text-chunker search manuscript.md "bhairava"
```

Case-insensitive search. Only content lines are searched (markers are excluded). Results are grouped by page number.

### Split into individual files

```bash
text-chunker split manuscript.md --outdir ./pages
```

Writes each page as `page-000.md`, `page-001.md`, etc. into the output directory (created if it doesn't exist).

### Chunk markdown into structural segments

```bash
text-chunker chunks manuscript.md              # document-level chunking
text-chunker chunks manuscript.md --per-page   # chunk within each page separately
```

**Document mode** (default) treats the entire file as continuous markdown — no page markers required. Works on any `.md` file. **Per-page mode** requires page markers; it chunks each page independently so heading context doesn't bleed across pages.

```
#      Type         Lines          Text
------------------------------------------------------------------------
1      heading      1-1            Chapter 1
2      paragraph    3-3            The quick brown fox jumps over the...
3      list_item    5-5            First item
4      list_item    6-6            Second item
5      code_block   8-10           let x = 1;
------------------------------------------------------------------------
Total: 5 chunks
```

Each chunk includes:
- **text** — clean extracted text (markdown syntax stripped)
- **chunk_type** — `heading`, `paragraph`, `list_item`, `code_block`, `table`, or `block_quote`
- **heading_context** — ancestor heading hierarchy, e.g. `["Chapter 1", "Background"]`
- **source_line_start / source_line_end** — 1-based source line numbers
- **page_number** — set in per-page mode

### JSON output

Add `--json` to any subcommand for machine-readable output:

```bash
text-chunker --json pages manuscript.md
text-chunker --json lines manuscript.md 5
text-chunker --json search manuscript.md "śiva"
text-chunker --json chunks manuscript.md
text-chunker --json chunks manuscript.md --per-page
```

### Stdin

```bash
cat manuscript.md | text-chunker pages -
```

Extension validation is skipped when reading from stdin.

## Tips: composing with other tools

`text-chunker` is designed to replace fragile `sed -n 'X,Yp'` range calculations by handling page boundaries for you. It composes naturally with other CLI tools:

```bash
# Copy page content to clipboard
text-chunker lines file.md 5 | pbcopy

# Search within a specific page
text-chunker lines file.md 5 | grep "śiva"

# Word count of a page
text-chunker lines file.md 5 | wc -w

# Extract lines as JSON array via jq
text-chunker --json lines file.md 5 | jq -r '.lines[]'

# Pre-filter a file, then chunk
cat file.md | sed 's/foo/bar/g' | text-chunker lines - 5

# Pipe page content to an LLM
text-chunker lines file.md 5 | claude -p "Translate this Sanskrit"
```

Quick shell function for repeated use:

```bash
page() { text-chunker lines "$1" "$2"; }
page manuscript.md 5
page manuscript.md 5-10
```

## MCP Server

Start the MCP server over stdio:

```bash
text-chunker mcp
```

This exposes five tools to any MCP-compatible client:

| Tool | Description |
|------|-------------|
| `pages` | List all pages with metadata (page numbers, image counts, line ranges, content line counts) |
| `lines` | Get lines from a page or range, with optional `mode` (`content`, `raw`, `no_markers`) |
| `search` | Case-insensitive substring search across all pages |
| `chunks` | Chunk markdown into structural segments for embedding, with optional `per_page` mode |
| `split` | Split pages into individual files in a given output directory |

All tools accept a `file` parameter (path to a `.md` or `.txt` file) and return JSON.

### Claude Desktop / MCP client config

```json
{
  "mcpServers": {
    "text-chunker": {
      "type": "stdio",
      "command": "text-chunker",
      "args": ["mcp"]
    }
  }
}
```

### MCP Inspector

```bash
npx @modelcontextprotocol/inspector cargo run --release -- mcp
```

### Smoke test

```bash
cargo build --release
uv run smoke_test.py
```

**Phase 1 (CLI)** — always runs, no API key needed. Invokes all five subcommands (`pages`, `lines`, `search`, `chunks`, `split`) via `subprocess` and validates output, including GFM/Obsidian extension handling (math, footnotes, task lists, wikilinks, definition lists, highlight/comment/block-anchor stripping).

**Phase 2 (MCP)** — runs only when `ANTHROPIC_API_KEY` is set. A Python agent (Claude Agent SDK) calls all five MCP tools over stdio and verifies each was exercised.

## Testing

```bash
cargo test
```

108 tests covering parsing, search, structural chunking, output formatting, JSON serialisation, error handling, line modes (`--raw`, `--no-markers`), MCP handler functions, and edge cases (Windows line endings, Unicode search, non-sequential pages, duplicate markers, heading context isolation, etc.).

## License

MIT
