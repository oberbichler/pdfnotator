# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Windows install via Scoop: `scoop bucket add oberbichler https://github.com/oberbichler/scoop-bucket`, then
  `scoop install oberbichler/pdfnotator`. Releases update the manifest automatically.

## [0.2.0] - 2026-11-14

### Added

- Draw tool (`D`): sketch on the page with freehand strokes, arrows and rectangles (`⇥` switches, `⌘Z` undoes),
  then start typing to comment. The agent gets the screenshot with the sketch in red plus a description of each
  shape — the words it encircles or points at and its position and size in mm — and, via `get_comments`, the
  region without the sketch.
- Tabs: several documents at once, each with its own comments, agent thread, zoom and scroll position. Agent
  sessions are bound to the document they opened; `wait_for_feedback`, `reply` and `viewer_status` accept an
  optional `path`. Closing a tab ends the agent's feedback loop.
- Optional left sidebar with page thumbnails (with open-comment badges) and the PDF outline (`⌥⌘S`).
- Find in document (`⌘F`, `⌘G`/`⇧⌘G`), ignoring case, accents and line breaks.

- “Send again” on sent comments, as a manual fallback if a delivery got lost.
- `get_comments` MCP tool: full quotes, surrounding page text and full-resolution screenshots on demand.

### Changed

- Selecting text opens the comment box immediately with the cursor in the input field (no extra click).
- MCP results are compact by default to save tokens: small screenshot previews (≤ 640 px), quotes, area text
  and compile errors are shortened (with a hint to `get_comments`), `viewer_status` is plain text, tool
  descriptions are shorter, tools carry `readOnlyHint` and related annotations.

### Fixed

- Comments could get lost and stop reaching the agent until PDFnotator was restarted: when an agent cancelled
  `wait_for_feedback` (e.g. Copilot's tool timeout), the call kept waiting in the viewer and swallowed later
  comments. Cancellations (`notifications/cancelled`) are now forwarded by the stdio bridge, only the newest wait
  per document can take comments, and cancelled calls get no response.
- `typst watch` kept running when PDFnotator was terminated by a signal.
- The comment box kept the text of the previous comment.

## [0.1.0] - 2026-11-13

### Added

- PDF viewer with continuous scrolling, pinch-to-zoom around the cursor and seamless in-place reload that keeps
  scroll position, zoom and active text selections.
- Built-in `typst watch` for `.typ` sources with compile errors shown in the viewer and reported to the agent.
- Text comments with _Comment_, _Replace_ and _Delete_ actions; area comments with high-resolution screenshots and
  the text inside the region.
- Feedback thread with status per comment (draft → sent → delivered → resolved) and agent replies; resolved
  comments are hidden on the page until selected.
- MCP server with `open_pdf`, `wait_for_feedback`, `reply` and `viewer_status` tools and a `review` prompt —
  via stdio (`pdfnotator --mcp`, launches the viewer on demand) or streamable HTTP on `127.0.0.1:4747`.
- Connect menu that copies a ready-made VS Code / Claude Code configuration and a start prompt.
- User interface in all 24 official EU languages.
- File associations for `.typ` and `.pdf`, single-instance hand-over from the command line.

[Unreleased]: ../../compare/v0.2.0...HEAD
[0.2.0]: ../../compare/v0.1.0...v0.2.0
[0.1.0]: ../../releases/tag/v0.1.0
