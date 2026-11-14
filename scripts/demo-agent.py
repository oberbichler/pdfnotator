#!/usr/bin/env python3
"""Scripted MCP agent for the demo recording (see record-demo.sh).

Talks to a running PDFnotator over streamable HTTP exactly like a real agent:
waits for feedback, edits the Typst source and replies. Standard library only.

usage: demo-agent.py <port> <document.typ> <done-marker>
"""

import json
import re
import sys
import time
import urllib.request

PORT, DOC, DONE = int(sys.argv[1]), sys.argv[2], sys.argv[3]
URL = f"http://127.0.0.1:{PORT}/mcp"
HEADERS = {"Content-Type": "application/json", "mcp-session-id": "demo-agent"}


def call(tool: str, **args) -> str:
    body = {"jsonrpc": "2.0", "id": int(time.time() * 1000), "method": "tools/call",
            "params": {"name": tool, "arguments": {"path": DOC, **args}}}
    req = urllib.request.Request(URL, json.dumps(body).encode(), HEADERS)
    with urllib.request.urlopen(req, timeout=700) as resp:
        result = json.load(resp)["result"]
    return "\n".join(c.get("text", "") for c in result["content"])


def wait_for_comment() -> str:
    while True:
        text = call("wait_for_feedback", timeout_seconds=600)
        ids = re.findall(r"\[(c\d+)\]", text)
        if ids:
            print(text, file=sys.stderr)
            return ids[0]


def edit(old: str, new: str) -> None:
    with open(DOC, encoding="utf-8") as f:
        source = f.read()
    assert old in source, old
    with open(DOC, "w", encoding="utf-8") as f:
        f.write(source.replace(old, new, 1))


COSTS = """Running costs drop from 41 k€ to 29 k€ per month once phase 3 is complete. The
one-off effort is estimated at 380 person-days, including training for the
operations team. Faster releases and on-demand scaling are not included in this
figure but are expected to be the larger benefit."""

COST_TABLE = """#table(
  columns: (1fr, auto, auto),
  inset: 7pt,
  stroke: 0.5pt + luma(180),
  fill: (_, y) => if y == 0 { luma(235) },
  [*Cost item*], [*Today*], [*After phase 3*],
  [Running costs per month], [41 k€], [29 k€],
  [One-off migration effort], [—], [380 person-days],
)
Faster releases and on-demand scaling are not included and are expected to be the larger benefit."""

# 1 · text comment → add a footnote
first = wait_for_comment()
time.sleep(1.4)  # "thinking"
edit("roughly 30 %", "roughly 30 %#footnote[Cost model v2, appendix A: hosting, licences and operations.]")
time.sleep(1.2)
call("reply", message="Added the source as a footnote.", resolved_ids=[first])

# 2 · area comment → turn the paragraph into a table
second = wait_for_comment()
time.sleep(1.4)
edit(COSTS, COST_TABLE)
time.sleep(1.2)
call("reply", message="Turned the cost figures into a table.", resolved_ids=[second])

# back to listening, like a real agent loop; the recorder stops shortly after
open(DONE, "w").close()
call("wait_for_feedback", timeout_seconds=30)
