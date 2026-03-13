# /// script
# requires-python = ">=3.12"
# dependencies = ["claude-agent-sdk>=0.1.21"]
# ///

"""
MCP Smoke Test -- Python Agent calling the text-chunker MCP server

Exercises all five MCP tools (pages, lines, search, chunks, split) against a
temporary fixture file with page markers.

Usage:
    1. Build first:
       cargo build --release

    2. Run the smoke test:
       uv run smoke_test.py
"""

import asyncio
import os
import tempfile
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
<!-- Page 0 - 2 images -->
The quick brown fox jumps over the lazy dog.
This is the first page of our test document.

<!-- Page 1 - 1 image -->
Pack my box with five dozen liquor jugs.
Sphinx of black quartz, judge my vow.

<!-- Page 2 - 3 images -->
How vexingly quick daft zebras jump.
The five boxing wizards jump quickly.
"""

TOOL_LABELS = {
    "pages": ("Listing pages", GREEN),
    "lines": ("Reading lines", CYAN),
    "search": ("Searching", YELLOW),
    "chunks": ("Chunking", MAGENTA),
    "split": ("Splitting", MAGENTA),
}


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

    # Write fixture to a temp .md file so the MCP server can read it
    with tempfile.NamedTemporaryFile(
        mode="w", suffix=".md", delete=False
    ) as f:
        f.write(FIXTURE)
        fixture_path = f.name

    split_dir = tempfile.mkdtemp(prefix="text-chunker-split-")

    try:
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
            "4) Chunk the document into structural segments, "
            f"5) Split the document into {split_dir}"
        )

        print(f"{BOLD}text-chunker MCP Smoke Test{RESET}")
        print(f"{DIM}Python agent (Claude Agent SDK) <-> Rust MCP server (rmcp + stdio){RESET}")
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

    finally:
        os.unlink(fixture_path)
        import shutil
        shutil.rmtree(split_dir, ignore_errors=True)


if __name__ == "__main__":
    asyncio.run(main())
