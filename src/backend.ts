/**
 * Typed bridge to the Rust backend (`src-tauri/src/commands.rs` and
 * `src-tauri/src/events.rs`).
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { FeedbackPayload } from "./lib/comments";

/** Mirrors `document::DocStatus`. */
export interface DocStatus {
  id: number;
  path: string | null;
  source: string | null;
  compileError: string | null;
  version: number;
  listeners: number;
  pending: number;
  currentPage: number;
  pageCount: number;
}

/** Mirrors `session::Status`. */
export interface ServerStatus {
  port: number;
  serverError: string | null;
  documents: DocStatus[];
}

/** Event payloads, keyed by event name. */
export interface BackendEvents {
  "pdf-opened": { doc: number; path: string; version: number };
  "pdf-changed": { doc: number; path: string; version: number };
  "doc-closed": { doc: number };
  focus: { doc: number };
  compile: { doc: number; ok: boolean; error: string | null };
  status: ServerStatus;
  "feedback-delivered": { doc: number; ids: string[] };
  "agent-message": { doc: number; message: string; resolved: string[] };
}

export interface AppInfo {
  version: string;
  /** Path of the running executable (for `<exe> --mcp`). */
  executable: string | null;
  /** `PDFNOTATOR_DEMO`: "1" stages a screenshot, "animate" plays the scripted demo. */
  demo: string | null;
}

export const backend = {
  appInfo: () => invoke<AppInfo>("app_info"),
  openDocument: (path: string) => invoke<number>("open_document", { path }),
  closeDocument: (doc: number) => invoke<void>("close_document", { doc }),
  readPdf: async (doc: number) => new Uint8Array(await invoke<ArrayBuffer>("read_pdf", { doc })),
  getStatus: () => invoke<ServerStatus>("get_status"),
  submitFeedback: (doc: number, items: FeedbackPayload[]) => invoke<void>("submit_feedback", { doc, items }),
  setView: (doc: number, page: number, pageCount: number) =>
    invoke<void>("set_view", { doc, page, pageCount }),
};

export function on<E extends keyof BackendEvents>(
  event: E,
  handler: (payload: BackendEvents[E]) => void,
): Promise<UnlistenFn> {
  return listen<BackendEvents[E]>(event, (e) => handler(e.payload));
}
