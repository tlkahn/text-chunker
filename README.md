# text-chunker

A Rust CLI tool for splitting page-marked documents into chunks. Built for OCR-processed Sanskrit manuscripts that use HTML comment markers (`<!-- Page N - M images -->`) to delimit pages.

Includes an MCP (Model Context Protocol) server so LLM agents can query pages, lines, and search results programmatically over stdio.

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

### JSON output

Add `--json` to any subcommand for machine-readable output:

```bash
text-chunker --json pages manuscript.md
text-chunker --json lines manuscript.md 5
text-chunker --json search manuscript.md "śiva"
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

This exposes three tools to any MCP-compatible client:

| Tool | Description |
|------|-------------|
| `pages` | List all pages with metadata (page numbers, image counts, line ranges, content line counts) |
| `lines` | Get lines from a page or range, with optional `mode` (`content`, `raw`, `no_markers`) |
| `search` | Case-insensitive substring search across all pages |

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

A Python smoke test exercises all three tools via the Claude Agent SDK:

```bash
cargo build --release
uv run smoke_test.py
```

## Testing

```bash
cargo test
```

76 tests covering parsing, search, output formatting, JSON serialisation, error handling, line modes (`--raw`, `--no-markers`), MCP handler functions, and edge cases (Windows line endings, Unicode search, non-sequential pages, duplicate markers, etc.).

## License

MIT
