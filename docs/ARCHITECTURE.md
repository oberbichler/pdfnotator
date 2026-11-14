# Architecture

PDFnotator is a [Tauri 2](https://v2.tauri.app) app: a Rust backend owns the document, file watching, Typst
compilation and the MCP server; a TypeScript frontend renders the PDF with [pdf.js](https://mozilla.github.io/pdf.js/)
and handles all interaction.

```
 ┌──────────────┐  stdio (JSON-RPC)   ┌──────────────────┐   HTTP (loopback)    ┌───────────────────────────┐
 │  AI agent    │ ──────────────────▶ │ pdfnotator --mcp │ ───────────────────▶ │  PDFnotator app           │
 │ (Copilot, …) │ ◀────────────────── │  (bridge.rs)     │ ◀─────────────────── │  http.rs → mcp/ → session │
 └──────────────┘                     └──────────────────┘   launches on demand └─────────────┬─────────────┘
                                                                                               │ Tauri events / commands
                                                                                     ┌─────────▼─────────┐
                                                                                     │  WebView (src/)   │
                                                                                     └───────────────────┘
```

## Backend (`src-tauri/src`)

| Module        | Responsibility                                                                                          |
| ------------- | ------------------------------------------------------------------------------------------------------- |
| `lib.rs`      | CLI parsing (`--mcp`, `--help`, file argument), single instance hand-over, Tauri setup                  |
| `session.rs`  | All open documents (tabs) and which agent session (`mcp-session-id`) works on which; routes tool calls  |
| `document.rs` | One document: `notify` file watcher with debounce, `typst watch`, feedback queue, status snapshot       |
| `feedback.rs` | `FeedbackItem` and `FeedbackQueue` — the hand-over between UI (producer) and agent (blocking consumer)  |
| `typst.rs`    | Spawns `typst watch`; `LogParser` turns its stderr into `CompileEvent`s (pure and unit tested)          |
| `mcp/`        | Transport-independent MCP: protocol negotiation, tools, prompts, rendering feedback for the agent       |
| `http.rs`     | axum router: `POST /mcp` (streamable HTTP, JSON responses) and the REST API, Host/Origin guard          |
| `bridge.rs`   | stdio transport. Answers `initialize`/`tools/list` locally, forwards tool calls, launches the viewer    |
| `events.rs`   | `EventSink` trait — the session emits UI events without depending on Tauri (tests use a recording sink) |
| `commands.rs` | Tauri commands called by the frontend                                                                   |

Design notes:

- **No Tauri in the core.** `Session` only knows `EventSink`, so the whole MCP flow (open → wait → reply) is tested
  in plain `cargo test`, including real `typst` compiles when the binary is installed.
- **Blocking waits are cheap.** `wait_for_feedback` parks on a `tokio::sync::Notify`; a short grace period batches
  comments sent in quick succession.
- **Tool errors are results.** Failures are returned as `isError: true` content (as recommended by the MCP spec), so
  the agent can read and react to them.
- **Seamless reload.** The watcher debounces events and waits for a stable file size before telling the UI.
- **Many agents, many documents.** The stdio bridge sends its own `mcp-session-id`, so each agent session is bound to
  the document it opened. Because one VS Code window shares a single MCP server process between chats, tool results
  also ask the agent to pass the document `path` explicitly — that always wins over the session binding.

## Frontend (`src`)

| Path         | Responsibility                                                                                                                                           |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `main.ts`    | Composition root: creates the `Viewer`, initialises UI modules, subscribes to backend events                                                             |
| `viewer.ts`  | pdf.js viewer: lazy page rendering, off-screen re-render and in-place swap on reload, zoom around a point                                                |
| `store.ts`   | Application state and actions; subscribers re-render once per microtask                                                                                  |
| `actions.ts` | Side-effecting user actions (send drafts, commit a comment)                                                                                              |
| `backend.ts` | Typed wrappers for Tauri commands and events                                                                                                             |
| `lib/`       | Pure logic: geometry, comment model and instruction rules, storage                                                                                       |
| `ui/`        | One module per UI area: tabs, navigator (thumbnails/outline), search, dock, selection bar, composer, area tool, marks, thread, sidebar, status, language |
| `i18n/`      | Typed messages (`en.ts` is the source of truth), plural rules via `Intl.PluralRules`, `data-i18n` binding                                                |

Data flow is one-directional: _backend event or user input → action → store → renderers_. Renderers never mutate
state directly.

## Feedback lifecycle

```
draft ──send──▶ sent ──agent fetches──▶ delivered ──reply(resolved_ids)──▶ resolved
```

Status only moves forward. Area comments are not sent before their screenshot is rendered. Resolved comments are
hidden on the page and only shown while selected in the sidebar.

## Testing

- `cargo test` — unit and integration tests for parser, queue, session (temp dirs, real file watching, real typst),
  MCP handling, HTTP router (via `tower::ServiceExt`) and the stdio bridge against a live server.
- `npm test` — Vitest: geometry, comment rules, store, actions (with mocked backend), composer and status in jsdom,
  and a locale validator that checks every language for missing keys, plural categories, placeholders and product
  names.
