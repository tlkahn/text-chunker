# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
cargo build                    # dev build
cargo build --release          # release build (needed for smoke test)
cargo test                     # run all 204 tests
cargo test test_name           # run a single test by name
cargo test mcp::               # run only MCP handler tests (18 tests)
cargo test latex               # run only LaTeX chunking tests
cargo test -- --nocapture      # show println output during tests
```

MCP smoke test (requires release build + ANTHROPIC_API_KEY):
```bash
uv run smoke_test.py
```

## Architecture

Three-file split with a strict separation of concerns:

- **src/lib.rs** — All domain logic and types. `ChunkerError`, `Page`, `SearchMatch`, `LineMode`, `LinesResult`, `Format`, `ChunkType` (including `MathBlock`, `Theorem`), and all public functions (`parse_pages`, `parse_page_range`, `search_pages`, `collect_page_lines`, `format_pages_table`, `format_search_results`, `split_pages`, `pages_to_json`, `lines_to_json`, `search_to_json`, `validate_extension`, `detect_format`, `chunk_markdown`, `chunk_latex`, `chunk_document_fmt`, `chunk_pages_fmt`). All 186 domain tests live here.
- **src/main.rs** — CLI only. `Cli` and `Commands` (clap derive), `main()`, `run()`, `read_input()`. Imports everything via `use text_chunker::*`. The `Mcp` subcommand short-circuits before file extraction.
- **src/mcp.rs** — MCP server. Testable content-based handlers (`handle_pages_content`, `handle_lines_content`, `handle_search_content`, `handle_chunks_content_fmt`) that take `&str` and return `Result<String, ChunkerError>`. File-reading wrappers (`handle_pages`, `handle_lines`, `handle_search`, `handle_chunks`) add validation + I/O + format detection. `TextChunkerMcp` struct uses rmcp's `#[tool_router]`/`#[tool_handler]` macros. 18 handler tests.

Key design decision: MCP handlers take `&str` content (not file paths) so they're unit-testable without touching the filesystem. The file-reading wrappers are thin and untested.

## rmcp Pitfalls

When modifying the MCP server, be aware of these rmcp 1.1 gotchas:

- **Must use `ServerCapabilities::builder().enable_tools().build()`** in `get_info()`. Using `ServerCapabilities::default()` produces empty `"capabilities":{}`, so MCP clients never discover tools.
- All model structs (`ServerInfo`, `ServerCapabilities`, `Implementation`) are `#[non_exhaustive]` — use builder/constructor methods, never struct literals.
- `ServerInfo` is a type alias for `InitializeResult`. Construct with `ServerInfo::new(capabilities).with_server_info(...).with_instructions(...)`.
- `Implementation::new(name, version)` — not struct literal.

## Page Marker Format

The regex is: `<!--\s*Page\s+(\d+)\s*-\s*(\d+)\s+images?\s*-->`

Content before the first marker is silently dropped. Page numbers can be non-sequential or duplicated (first match wins for lookups).
