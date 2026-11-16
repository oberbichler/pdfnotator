/**
 * Typed bridge to the Rust backend (`src-tauri/src/commands.rs` and
 * `src-tauri/src/events.rs`). In the desktop app it uses Tauri IPC; in a
 * tablet's browser (`src-tauri/src/remote.rs`) the same calls go over HTTP and
 * events arrive as server-sent events.
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
  /** Names of the agents waiting for feedback (as far as they told us). */
  agents: string[];
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

/** Pairing info for a tablet (`remote::RemoteInfo`). */
export interface RemoteInfo {
  url: string;
  qrSvg: string;
}

/** Running inside the desktop app (not in a tablet's browser). */
export const isDesktop = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function http<T>(method: "GET" | "POST", path: string, body?: unknown): Promise<T> {
  const resp = await fetch(`/remote/${path}`, {
    method,
    credentials: "same-origin",
    headers: body === undefined ? undefined : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!resp.ok) throw new Error(`${resp.status} ${await resp.text()}`);
  const type = resp.headers.get("Content-Type") ?? "";
  if (type.includes("json")) return (await resp.json()) as T;
  if (type.includes("pdf")) return (await resp.arrayBuffer()) as T;
  return undefined as T;
}

/** The desktop-only parts are no-ops (or errors) in the browser. */
const remoteBackend = {
  appInfo: () => http<AppInfo>("GET", "app-info"),
  openDocument: (_path: string): Promise<number> => Promise.reject(new Error("open files on the computer")),
  closeDocument: (doc: number) => http<void>("POST", `close/${doc}`),
  readPdf: async (doc: number) => new Uint8Array(await http<ArrayBuffer>("GET", `pdf/${doc}`)),
  getStatus: () => http<ServerStatus>("GET", "status"),
  submitFeedback: (doc: number, items: FeedbackPayload[]) => http<void>("POST", `feedback/${doc}`, items),
  setView: (doc: number, page: number, pageCount: number) =>
    http<void>("POST", `view/${doc}`, { page, pageCount }),
  remoteStart: (): Promise<RemoteInfo> => Promise.reject(new Error("desktop only")),
  remoteStop: () => Promise.resolve(),
  remoteInfo: () => Promise.resolve(null as RemoteInfo | null),
};

const desktopBackend: typeof remoteBackend = {
  appInfo: () => invoke<AppInfo>("app_info"),
  openDocument: (path: string) => invoke<number>("open_document", { path }),
  closeDocument: (doc: number) => invoke<void>("close_document", { doc }),
  readPdf: async (doc: number) => new Uint8Array(await invoke<ArrayBuffer>("read_pdf", { doc })),
  getStatus: () => invoke<ServerStatus>("get_status"),
  submitFeedback: (doc: number, items: FeedbackPayload[]) => invoke<void>("submit_feedback", { doc, items }),
  setView: (doc: number, page: number, pageCount: number) =>
    invoke<void>("set_view", { doc, page, pageCount }),
  remoteStart: () => invoke<RemoteInfo>("remote_start"),
  remoteStop: () => invoke<void>("remote_stop"),
  remoteInfo: () => invoke<RemoteInfo | null>("remote_info"),
};

export const backend = isDesktop ? desktopBackend : remoteBackend;

/** One `EventSource` for all remote listeners; reconnects by itself. */
let source: EventSource | null = null;

export function on<E extends keyof BackendEvents>(
  event: E,
  handler: (payload: BackendEvents[E]) => void,
): Promise<UnlistenFn> {
  if (isDesktop) return listen<BackendEvents[E]>(event, (e) => handler(e.payload));
  source ??= new EventSource("/remote/events");
  const es = source;
  const listener = (e: MessageEvent<string>) => handler(JSON.parse(e.data) as BackendEvents[E]);
  es.addEventListener(event, listener);
  return Promise.resolve(() => es.removeEventListener(event, listener));
}
