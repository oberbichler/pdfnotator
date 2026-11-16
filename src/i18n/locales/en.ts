/**
 * English UI strings — the source of truth for all other locales.
 *
 * Conventions:
 * - `{name}` placeholders are replaced at runtime and must be kept verbatim.
 * - Plural messages are objects keyed by `Intl.PluralRules` categories
 *   (`zero`, `one`, `two`, `few`, `many`, `other`); `other` is required.
 * - Keyboard symbols (⌘, ↵, ⇧, ⇥, Esc) and product names stay untranslated.
 */
const en = {
  "app.name": "PDFnotator",

  "agent.none": "No agent",
  "agent.noneHint": "Tell your agent “open in pdfnotator” or paste the start prompt.",
  "agent.listening": "Agent listening",
  "agent.listeningAs": "{name} listening",
  "agent.listeningHint": "Comments reach the agent immediately.",
  "agent.working": "Agent working",
  "agent.workingHint": "New comments are collected and handed over with the next fetch.",
  "agent.serverError": "Server error",
  "agent.pending": { one: "{n} comment waiting to be fetched", other: "{n} comments waiting to be fetched" },
  "agent.name": "Agent",

  "connect.copyUrl": "Copy URL",
  "connect.copyPrompt": "Copy start prompt",
  "connect.copyPromptHint": "Paste into your agent chat",
  "connect.copyVscode": "Copy VS Code mcp.json",
  "connect.copyVscodeHint": "Connect Copilot once",
  "connect.copyClaude": "Copy Claude Code command",
  "connect.copyClaudeHint": "claude mcp add …",
  "connect.copyCopilotCli": "Copy Copilot CLI command",
  "connect.copyCopilotCliHint": "copilot mcp add …",
  "connect.copied": "Copied",
  "connect.promptCopied": "Prompt copied — paste it into your agent chat",
  "connect.copyFailed": "Copy failed: {error}",

  "language.title": "Language",
  "language.system": "System default",

  "empty.title": "Drop a document here",
  "empty.subtitle": ".typ or .pdf — or tell your agent “open in pdfnotator”",
  "empty.open": "Open file",
  "dialog.filter": "Typst / PDF",

  "tool.text": "Select text",
  "tool.area": "Mark area",
  "tool.draw": "Draw",
  "shape.pen": "Freehand",
  "shape.arrow": "Arrow",
  "shape.rect": "Rectangle",
  "shape.text": "Text",
  "zoom.in": "Zoom in",
  "zoom.out": "Zoom out",
  "zoom.fit": "Fit width",
  "page.jump": "Go to page",

  "sidebar.title": "Feedback",
  "sidebar.toggle": "Toggle sidebar",
  "resize.handle": "Drag to resize · double-click to reset",
  "resolved.hide": "Hide resolved",
  "resolved.show": "Show resolved",
  "hint.text": "Select text",
  "hint.area": "Mark an area",
  "hint.draw": "Draw on the page",
  "hint.delete": "Delete selection",
  "hint.body": "Every comment goes straight to the agent — the PDF updates by itself.",

  "drafts.count": { one: "{n} draft", other: "{n} drafts" },
  "drafts.sendAll": "Send all",
  "general.placeholder": "Message to the agent …",
  "live.label": "Instant",
  "live.on": "Instant send is on: every comment goes straight to the agent",
  "live.off": "Instant send is off: collect comments, then send them all with ⌘↵",
  send: "Send",

  "action.comment": "Comment",
  "action.replace": "Replace",
  "action.delete": "Delete",

  "composer.cancel": "Cancel",
  "composer.placeholder.comment": "What should change?",
  "composer.placeholder.replace": "New text …",
  "composer.placeholder.delete": "Reason (optional) …",
  "composer.verb.send": "send",
  "composer.verb.add": "add",
  "composer.verb.delete": "delete",
  "composer.mode": "mode",
  "composer.area": "Area · p. {page}",
  "composer.sketch": "Sketch · p. {page}",
  "sketch.hint": "Type to comment · ⌘Z undo · Esc discard",

  "comment.deleteText": "Delete this text.",
  "comment.deleteArea": "Delete this area.",
  "comment.replaceWith": "Replace with: «{text}»",

  "card.general": "General",
  "card.page": "p. {page}",
  "card.noComment": "no comment",
  "card.delete": "Delete",
  "card.resend": "Send again",
  "status.draft": "draft",
  "status.draftTitle": "Not sent yet",
  "status.sent": "sent",
  "status.sentTitle": "Waiting for the agent to fetch it",
  "status.delivered": "with agent",
  "status.deliveredTitle": "The agent received this comment",
  "status.resolved": "done",
  "status.resolvedTitle": "Marked as done by the agent",

  "tab.close": "Close tab",
  "tab.new": "Open document",
  "tab.closedByUser": "Document closed",

  "nav.toggle": "Pages & outline",
  "nav.pages": "Pages",
  "nav.outline": "Outline",
  "nav.noOutline": "This document has no outline",
  "nav.page": "Page {page}",
  "nav.comments": { one: "{n} open comment", other: "{n} open comments" },

  "search.open": "Find",
  "search.placeholder": "Find in document",
  "search.count": "{current} of {total}",
  "search.none": "No matches",
  "search.next": "Next match",
  "search.prev": "Previous match",
  "search.close": "Close search",

  "compile.title": "Typst error",
  "compile.hint": "showing the last good version",
  "live.typst": "Live — pdfnotator compiles {file} automatically",
  "live.pdf": "Live — reloads automatically when the file changes",

  "error.loadPdf": "Could not load the PDF: {error}",
  "error.screenshot": "Screenshot failed: {error}",
  "error.send": "Sending failed: {error}",
  "error.unsupportedDrop": "Only .typ and .pdf files are supported",
} as const;

/** A plural message keyed by `Intl.PluralRules` category. */
export type Plural = Partial<Record<Intl.LDMLPluralRule, string>> & { other: string };

/** Shape every locale must implement: same keys, strings or plurals. */
export type Messages = {
  [K in keyof typeof en]: (typeof en)[K] extends string ? string : Plural;
};

export type MessageKey = keyof Messages;

export default en satisfies Messages;
