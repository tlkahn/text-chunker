# /// script
# requires-python = ">=3.12"
# dependencies = ["claude-agent-sdk>=0.1.21"]
# ///

"""
Smoke Test -- CLI + MCP smoke tests for text-chunker

Phase 1 (CLI): Runs 5 CLI commands via subprocess and validates output.
               No API key needed.
Phase 2 (MCP): Python Agent calling the text-chunker MCP server.
               Requires ANTHROPIC_API_KEY.

Usage:
    1. Build first:
       cargo build --release

    2. Run the smoke test:
       uv run smoke_test.py
"""

import asyncio
import json
import os
import subprocess
import tempfile
from pathlib import Path
from claude_agent_sdk import (
    ClaudeAgentOptions,
    query,
    AssistantMessage,
    ResultMessage,
    TextBlock,
    ToolUseBlock,
    ToolResultBlock,
)

# ANSI helpers
DIM = "\033[2m"
BOLD = "\033[1m"
GREEN = "\033[32m"
YELLOW = "\033[33m"
CYAN = "\033[36m"
MAGENTA = "\033[35m"
RESET = "\033[0m"

BINARY = os.path.join(
    os.path.dirname(os.path.abspath(__file__)),
    "target",
    "release",
    "text-chunker",
)

FIXTURE = """\
---
title: Smoke Test Document
tags: [test, mcp]
---

<!-- Page 0 - 2 images -->
The quick brown fox jumps over the lazy dog.
This is the first page of our test document.

The equation $E = mc^2$ is famous[^1].

[^1]: Einstein's mass-energy equivalence.

<!-- Page 1 - 1 image -->
Pack my box with five dozen liquor jugs.
Sphinx of black quartz, judge my vow.

- [ ] unchecked task
- [x] checked task

See [[Related Page]] for more details.

<!-- Page 2 - 3 images -->
How vexingly quick daft zebras jump. %%hidden comment%%
The five boxing wizards jump quickly.

Term One
:   Definition of term one

This has ==highlighted text== inside. ^block-anchor
"""

TOOL_LABELS = {
    "pages": ("Listing pages", GREEN),
    "lines": ("Reading lines", CYAN),
    "search": ("Searching", YELLOW),
    "chunks": ("Chunking", MAGENTA),
    "split": ("Splitting", MAGENTA),
}


RED = "\033[31m"


def validate_chunks(json_str: str) -> list[str]:
    """Return list of failure messages (empty = pass)."""
    data = json.loads(json_str)
    texts = [c["text"] for c in data["chunks"]]
    types = [c["chunk_type"] for c in data["chunks"]]
    joined = " ".join(texts)

    failures = []
    if "E = mc^2" not in joined:
        failures.append("math lost")
    if "[^1]" not in joined:
        failures.append("footnote ref lost")
    if not any("Einstein" in t for t in texts):
        failures.append("footnote def lost")
    if not any(t.startswith("[ ] ") for t in texts):
        failures.append("unchecked task missing")
    if not any(t.startswith("[x] ") for t in texts):
        failures.append("checked task missing")
    if "Related Page" not in joined:
        failures.append("wikilink text lost")
    if "[[" in joined:
        failures.append("wikilink brackets not stripped")
    if "hidden comment" in joined:
        failures.append("obsidian comment not stripped")
    if "highlighted text" not in joined:
        failures.append("highlight text lost")
    if "==" in joined:
        failures.append("highlight markers not stripped")
    if "^block-anchor" in joined:
        failures.append("block anchor not stripped")
    if "title:" in joined:
        failures.append("frontmatter not stripped")
    if "definition_item" not in types:
        failures.append("definition_item type missing")
    return failures


def run_cli_smoke_test(binary: str, fixture_path: str, split_dir: str) -> bool:
    """Run CLI commands and validate output. Returns True if all pass."""
    passed = 0
    failed = 0
    total_checks = 5

    def check(name: str, args: list[str], validator, detail_fn=None):
        nonlocal passed, failed
        result = subprocess.run(args, capture_output=True, text=True)
        if result.returncode != 0:
            failed += 1
            print(f"  {RED}[{name}] FAIL: exit code {result.returncode}{RESET}")
            if result.stderr:
                print(f"    {DIM}{result.stderr.strip()}{RESET}")
            return
        ok = validator(result.stdout)
        if isinstance(ok, bool):
            if ok:
                passed += 1
                print(f"  {GREEN}[{name}] PASS{RESET}")
            else:
                failed += 1
                print(f"  {RED}[{name}] FAIL{RESET}")
        elif isinstance(ok, list):
            # list of failure messages
            if not ok:
                passed += 1
                count = detail_fn() if detail_fn else ""
                print(f"  {GREEN}[{name}] PASS{count}{RESET}")
            else:
                failed += 1
                print(f"  {RED}[{name}] FAIL: {', '.join(ok)}{RESET}")

    # 1. pages --json
    check("pages", [binary, "--json", "pages", fixture_path],
          lambda out: '"total_pages": 3' in out)

    # 2. lines
    check("lines", [binary, "lines", fixture_path, "1"],
          lambda out: "Pack my box" in out)

    # 3. search --json
    check("search", [binary, "--json", "search", fixture_path, "quick"],
          lambda out: '"total_matches"' in out and json.loads(out).get("total_matches", 0) >= 2)

    # 4. chunks --json (most assertions)
    num_chunk_checks = 13

    check("chunks", [binary, "--json", "chunks", fixture_path],
          lambda out: validate_chunks(out),
          detail_fn=lambda: f" ({num_chunk_checks}/{num_chunk_checks} checks)")

    # 5. split
    check("split", [binary, "split", fixture_path, "--outdir", split_dir],
          lambda out: Path(split_dir, "page-000.md").exists())

    print(f"\n  CLI: {passed}/{total_checks} passed")
    return failed == 0


def format_tool_call(block):
    short_name = block.name.split("__")[-1] if "__" in block.name else block.name
    label, color = TOOL_LABELS.get(short_name, (short_name, DIM))
    detail = ""
    if isinstance(block.input, dict):
        vals = [f"{v}" for v in block.input.values()]
        if vals:
            detail = f" -> {', '.join(vals)}"
    return f"{color}  [{label}{detail}]{RESET}"


def format_tool_result(block):
    content = block.content if isinstance(block.content, str) else str(block.content)
    if not content or not content.strip():
        return None
    # Truncate long JSON results for readability
    lines = content.strip().split("\n")
    if len(lines) > 6:
        preview = "\n".join(lines[:6]) + f"\n  ... ({len(lines)} lines total)"
    else:
        preview = content.strip()
    return f"  {DIM}| {preview}{RESET}"


async def main():
    if not os.path.exists(BINARY):
        print(f"Error: Binary not found at {BINARY}")
        print("Build it first: cargo build --release")
        return

    # Write fixture to a temp .md file
    with tempfile.NamedTemporaryFile(
        mode="w", suffix=".md", delete=False
    ) as f:
        f.write(FIXTURE)
        fixture_path = f.name

    split_dir = tempfile.mkdtemp(prefix="text-chunker-split-")

    try:
        # Phase 1: CLI smoke test (no API key needed)
        print(f"{BOLD}Phase 1: CLI Smoke Test{RESET}")
        cli_ok = run_cli_smoke_test(BINARY, fixture_path, split_dir)

        # Phase 2: MCP smoke test (needs ANTHROPIC_API_KEY)
        if not os.environ.get("ANTHROPIC_API_KEY"):
            print(f"\n{DIM}Skipping MCP smoke test (no ANTHROPIC_API_KEY){RESET}")
        else:
            print(f"\n{BOLD}Phase 2: MCP Smoke Test{RESET}")
            print(f"{DIM}Python agent (Claude Agent SDK) <-> Rust MCP server (rmcp + stdio){RESET}")

            # Recreate split_dir since CLI phase used it
            import shutil
            shutil.rmtree(split_dir, ignore_errors=True)
            os.makedirs(split_dir)

            options = ClaudeAgentOptions(
                model="claude-haiku-4-5",
                system_prompt=f"""You are a helpful assistant with access to a text-chunker MCP server
for querying page-marked manuscripts.

Available tools:
- pages(file) -- list all pages with metadata
- lines(file, page, mode?) -- get lines from a page or range
- search(file, term) -- search for text across all pages
- chunks(file, per_page?) -- chunk markdown into structural segments for embedding
- split(file, outdir) -- split into individual page files

The test document is at: {fixture_path}
The output directory for split is: {split_dir}

IMPORTANT: You must call ALL FIVE tools (pages, lines, search, chunks, split) to
answer the user's question. Do not skip any tool calls.""",
                mcp_servers={
                    "text-chunker": {
                        "type": "stdio",
                        "command": BINARY,
                        "args": ["mcp"],
                    }
                },
                allowed_tools=["mcp__text-chunker__*"],
                permission_mode="bypassPermissions",
            )

            prompt = (
                f"Using the file at {fixture_path}: "
                "1) List all pages, "
                "2) Show the lines from page 1, "
                "3) Search for the word 'quick', "
                "4) Chunk the document into structural segments (check that math, footnotes, "
                "task lists, wikilinks, definition items, and Obsidian syntax are handled correctly "
                "-- frontmatter/comments/highlights/block-anchors should be stripped, text preserved), "
                f"5) Split the document into {split_dir}"
            )

            print(f"\n{BOLD}Prompt:{RESET} {prompt}\n")

            tools_called = set()

            async for message in query(prompt=prompt, options=options):
                if isinstance(message, AssistantMessage):
                    for block in message.content:
                        if isinstance(block, TextBlock):
                            print(block.text, end="", flush=True)
                        elif isinstance(block, ToolUseBlock):
                            short = block.name.split("__")[-1] if "__" in block.name else block.name
                            tools_called.add(short)
                            print(f"\n{format_tool_call(block)}", flush=True)
                        elif isinstance(block, ToolResultBlock):
                            result = format_tool_result(block)
                            if result:
                                print(result, flush=True)

                elif isinstance(message, ResultMessage):
                    cost = message.total_cost_usd
                    turns = message.num_turns
                    print(f"\n{DIM}  ({turns} turns, ${cost:.4f}){RESET}")

            # Verify all five tools were exercised
            expected = {"pages", "lines", "search", "chunks", "split"}
            missing = expected - tools_called
            if missing:
                print(f"\n{BOLD}FAIL:{RESET} Tools not called: {missing}")
            else:
                print(f"\n{GREEN}{BOLD}PASS:{RESET} All five tools exercised ({', '.join(sorted(tools_called))})")

        if not cli_ok:
            raise SystemExit(1)

    finally:
        os.unlink(fixture_path)
        import shutil
        shutil.rmtree(split_dir, ignore_errors=True)


if __name__ == "__main__":
    asyncio.run(main())
