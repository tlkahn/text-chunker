#!/usr/bin/env python3
# /// script
# requires-python = ">=3.12"
# dependencies = ["claude-agent-sdk>=0.1.21"]
# ///

"""
Sanskrit Translation Automation

Translates pages from the Vijñānabhairava commentary using Claude agents
with the text-chunker MCP server.

Workflow:
  1. Split target page range into individual markdown files
  2. Translate each page sequentially (using previous page as example)
  3. Join all translated files back into the source document

Usage:
    cargo build --release
    uv run translate.py
    uv run translate.py --start 101 --end 110
    uv run translate.py --join-only
    uv run translate.py --dry-run
"""

import argparse
import asyncio
import os
import re
import sys
from pathlib import Path

from claude_agent_sdk import (
    ClaudeAgentOptions,
    query,
    AssistantMessage,
    ResultMessage,
    TextBlock,
    ToolUseBlock,
)

# -- ANSI helpers -------------------------------------------------------------

DIM = "\033[2m"
BOLD = "\033[1m"
GREEN = "\033[32m"
YELLOW = "\033[33m"
CYAN = "\033[36m"
RED = "\033[31m"
RESET = "\033[0m"

# -- Defaults -----------------------------------------------------------------

SOURCE_FILE = (
    "/Users/toeinriver/Documents/Ekuro/KSTS-242056-Vijnanabhairavatantra.md"
)
BINARY = os.path.join(
    os.path.dirname(os.path.abspath(__file__)),
    "target",
    "release",
    "text-chunker",
)
DEFAULT_MODEL = "claude-sonnet-4-6"
START_PAGE = 101
END_PAGE = 144
WORK_DIR = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "translation-work"
)

# -- Patterns -----------------------------------------------------------------

PAGE_MARKER_RE = re.compile(r"<!--\s*Page\s+(\d+)\s*-\s*(\d+)\s+images?\s*-->")
UNDERLINE_LINE = r"$\underline{\qquad\qquad\qquad\qquad}$"
TRANSLATION_OPEN = "<!-- [!translation]"


# -- Document manipulation ----------------------------------------------------


def extract_pages(source: str) -> dict[int, str]:
    """Parse source file into {page_number: full_content_up_to_next_marker}."""
    content = Path(source).read_text()
    pages: dict[int, str] = {}
    markers = list(PAGE_MARKER_RE.finditer(content))
    for i, m in enumerate(markers):
        page_num = int(m.group(1))
        start = m.start()
        end = markers[i + 1].start() if i + 1 < len(markers) else len(content)
        pages[page_num] = content[start:end]
    return pages


def split_to_files(
    pages: dict[int, str], start: int, end: int, outdir: str
) -> list[Path]:
    """Write pages [start..end] as individual files in outdir."""
    os.makedirs(outdir, exist_ok=True)
    files: list[Path] = []
    for pn in range(start, end + 1):
        if pn not in pages:
            print(f"  {YELLOW}Warning: Page {pn} not found{RESET}")
            continue
        p = Path(outdir) / f"page-{pn:03d}.md"
        p.write_text(pages[pn])
        files.append(p)
    return files


def content_above_underline(text: str) -> str:
    """Return page content above the latex underline, with page marker stripped.

    Includes original Sanskrit text and any existing translation block.
    Excludes the critical apparatus (footnotes below the underline).
    """
    text = PAGE_MARKER_RE.sub("", text, count=1).lstrip("\n")
    idx = text.find(UNDERLINE_LINE)
    if idx >= 0:
        return text[:idx].rstrip()
    return text.rstrip()


def has_translation(text: str) -> bool:
    """Check whether a page already contains a <!-- [!translation] block."""
    return TRANSLATION_OPEN in text


def insert_translation(page_content: str, translation: str) -> str:
    """Insert a <!-- [!translation] ... --> block before the underline.

    If no underline exists, append at the end of the page content.
    """
    block = f"\n{TRANSLATION_OPEN}\n\n{translation}\n-->\n"
    idx = page_content.find(UNDERLINE_LINE)
    if idx >= 0:
        return page_content[:idx] + block + "\n" + page_content[idx:]
    # No underline — append before trailing whitespace
    return page_content.rstrip("\n") + "\n" + block + "\n"


def join_translated(
    pages: dict[int, str],
    start: int,
    end: int,
    outdir: str,
    source: str,
):
    """Replace pages [start..end] in the source file with translated versions."""
    content = Path(source).read_text()

    # Build marker position map
    markers = list(PAGE_MARKER_RE.finditer(content))
    marker_map = {int(m.group(1)): m.start() for m in markers}

    region_start = marker_map[start]

    # Find the first page after our range
    after = end + 1
    max_page = max(marker_map.keys())
    while after not in marker_map and after <= max_page + 1:
        after += 1
    region_end = marker_map.get(after, len(content))

    # Build replacement from work-dir files (fall back to in-memory pages)
    parts: list[str] = []
    for pn in range(start, end + 1):
        f = Path(outdir) / f"page-{pn:03d}.md"
        if f.exists():
            parts.append(f.read_text())
        elif pn in pages:
            parts.append(pages[pn])
    replacement = "".join(parts)

    new_content = content[:region_start] + replacement + content[region_end:]

    # Backup original (only once)
    backup = source + ".bak"
    if not Path(backup).exists():
        Path(backup).write_text(content)
        print(f"  Backed up original to {backup}")

    Path(source).write_text(new_content)


# -- Translation agent --------------------------------------------------------

SYSTEM_PROMPT_TEMPLATE = """\
You are an expert translator of Sanskrit śāstra literature, specializing in \
Kashmiri Śaiva texts (Trika, Pratyabhijñā, Spanda). You translate the \
scholarly Sanskrit commentary of Śivopādhyāya on the Vijñānabhairava with \
philological precision.

## Output format

Return ONLY the English translation text. Do NOT include the \
`<!-- [!translation]` opener or `-->` closer — the automation script adds those.

## Translation conventions

1. Translate the prose commentary sentence by sentence. Provide Sanskrit \
compound analysis in parentheses using italics (*term*).
2. For verse quotations (marked `> [!quote]` or `> [!main]`), render as \
English blockquotes with quotation marks for citations and without for root verses.
3. Add footnotes `[^N]` (starting from [^1] per page) for: technical term \
explanations, philosophical context, textual variants, cross-references, \
OCR corrections.
4. Keep philosophically loaded Sanskrit terms in italics (*samādhi*, \
*vikalpa*, *bhairava*, etc.) when the English gloss alone would lose nuance.
5. Use IAST transliteration for all Sanskrit.
6. Where the commentary text is cut off mid-sentence at a page boundary, \
translate what is present and note the continuation.

## MCP context

The full manuscript is at: {source_file}
You have access to a text-chunker MCP server to look up surrounding context \
if needed:
- pages(file) — list all pages with metadata
- lines(file, page, mode?) — read lines from a page or range
- search(file, term) — full-text search across pages"""

USER_PROMPT_TEMPLATE = """\
{prev_context}

---

Now translate the given texts (sans the original footnotes, i.e. critical \
apparatus below the latex underline) like the example text above.

{main_text}"""


async def translate_page(
    page_file: Path,
    prev_context: str,
    source_file: str,
    model: str,
) -> str:
    """Translate a single page file using a Claude agent."""
    page_content = page_file.read_text()
    main_text = content_above_underline(page_content)

    system_prompt = SYSTEM_PROMPT_TEMPLATE.format(source_file=source_file)
    prompt = USER_PROMPT_TEMPLATE.format(
        prev_context=prev_context, main_text=main_text
    )

    options = ClaudeAgentOptions(
        model=model,
        system_prompt=system_prompt,
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

    parts: list[str] = []

    async for message in query(prompt=prompt, options=options):
        if isinstance(message, AssistantMessage):
            for block in message.content:
                if isinstance(block, TextBlock):
                    parts.append(block.text)
                elif isinstance(block, ToolUseBlock):
                    short = (
                        block.name.split("__")[-1]
                        if "__" in block.name
                        else block.name
                    )
                    print(f"    {DIM}[MCP: {short}]{RESET}", flush=True)
        elif isinstance(message, ResultMessage):
            cost = message.total_cost_usd
            turns = message.num_turns
            print(
                f"    {DIM}({turns} turns, ${cost:.4f}){RESET}",
                flush=True,
            )

    return "".join(parts)


# -- CLI ----------------------------------------------------------------------


def parse_args():
    parser = argparse.ArgumentParser(
        description="Translate Sanskrit commentary pages using Claude agents"
    )
    parser.add_argument(
        "--model",
        default=DEFAULT_MODEL,
        help=f"Claude model ID (default: {DEFAULT_MODEL})",
    )
    parser.add_argument(
        "--start",
        type=int,
        default=START_PAGE,
        help=f"First page to translate (default: {START_PAGE})",
    )
    parser.add_argument(
        "--end",
        type=int,
        default=END_PAGE,
        help=f"Last page to translate (default: {END_PAGE})",
    )
    parser.add_argument(
        "--source", default=SOURCE_FILE, help="Source markdown file"
    )
    parser.add_argument(
        "--join-only",
        action="store_true",
        help="Skip translation, just join existing work-dir files",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Split files only, do not translate",
    )
    return parser.parse_args()


async def main() -> int:
    args = parse_args()

    if not os.path.exists(BINARY):
        print(f"{RED}Error:{RESET} Binary not found at {BINARY}")
        print("Build first:  cargo build --release")
        return 1

    print(f"{BOLD}Sanskrit Translation Script{RESET}")
    print(
        f"{DIM}Model: {args.model}  |  Pages: {args.start}-{args.end}{RESET}\n"
    )

    # -- Parse source --
    print("Parsing source document...")
    pages = extract_pages(args.source)
    print(f"  {len(pages)} pages found")

    missing = [p for p in range(args.start, args.end + 1) if p not in pages]
    if missing:
        print(f"{RED}Error:{RESET} Missing pages: {missing}")
        return 1

    # -- Join-only mode --
    if args.join_only:
        print(f"\n{BOLD}Joining{RESET} work-dir files back into source")
        join_translated(pages, args.start, args.end, WORK_DIR, args.source)
        print(f"{GREEN}Done!{RESET}")
        return 0

    # -- Phase 1: Split --
    print(f"\n{BOLD}Phase 1:{RESET} Splitting pages {args.start}-{args.end}")
    files = split_to_files(pages, args.start, args.end, WORK_DIR)
    print(f"  {len(files)} files -> {WORK_DIR}")

    if args.dry_run:
        print(f"\n{YELLOW}Dry run — stopping before translation{RESET}")
        return 0

    # -- Phase 2: Translate --
    print(f"\n{BOLD}Phase 2:{RESET} Translating pages")

    for pn in range(args.start, args.end + 1):
        pf = Path(WORK_DIR) / f"page-{pn:03d}.md"
        if not pf.exists():
            continue

        current = pf.read_text()
        if has_translation(current):
            print(f"  Page {pn}: {DIM}already translated, skipping{RESET}")
            pages[pn] = current  # keep translated version in memory
            continue

        print(f"  Page {pn}: translating...")

        # Previous page context (original text + translation, no apparatus)
        prev_num = pn - 1
        prev_text = pages.get(prev_num, "")
        prev_context = content_above_underline(prev_text) if prev_text else ""

        try:
            translation = await translate_page(
                pf, prev_context, args.source, args.model
            )
        except Exception as e:
            print(f"  Page {pn}: {RED}ERROR — {e}{RESET}")
            continue

        if not translation.strip():
            print(f"  Page {pn}: {RED}WARNING — empty translation{RESET}")
            continue

        updated = insert_translation(current, translation)
        pf.write_text(updated)
        pages[pn] = updated
        print(f"  Page {pn}: {GREEN}done{RESET}")

    # -- Phase 3: Join --
    print(f"\n{BOLD}Phase 3:{RESET} Joining translated pages into source")
    join_translated(pages, args.start, args.end, WORK_DIR, args.source)
    print(f"\n{GREEN}{BOLD}All done!{RESET}")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
