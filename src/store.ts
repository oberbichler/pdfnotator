/**
 * Application state with a tiny subscription mechanism. Mutations go through
 * the exported actions; subscribers re-render after each change (batched per
 * microtask).
 *
 * Every open document (tab) has its own comments, agent replies, selection and
 * view; the server-side status of each document is merged in from `status`
 * events.
 */
import type { DocStatus, ServerStatus } from "./backend";
import type { AgentMessage, Comment, CommentStatus } from "./lib/comments";
import type { ViewState } from "./viewer";

export type Tool = "text" | "area";
export type NavTab = "pages" | "outline";

export interface DocState extends DocStatus {
  comments: Comment[];
  agentMessages: AgentMessage[];
  /** Comment selected in the sidebar (resolved marks are only shown when selected). */
  selectedId: string | null;
  /** An agent has worked on this document in this session. */
  agentSeen: boolean;
  view: ViewState | null;
}

export interface State {
  docs: DocState[];
  activeId: number | null;
  port: number;
  serverError: string | null;
  hideResolved: boolean;
  /** Send each comment immediately instead of collecting drafts. */
  live: boolean;
  tool: Tool;
  navOpen: boolean;
  navTab: NavTab;
}

export const state: State = {
  docs: [],
  activeId: null,
  port: 4747,
  serverError: null,
  hideResolved: false,
  live: true,
  tool: "text",
  navOpen: false,
  navTab: "pages",
};

type Listener = (s: State) => void;
const listeners = new Set<Listener>();
let scheduled = false;

/** Subscribe to state changes. Returns an unsubscribe function. */
export function subscribe(fn: Listener): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

/** Apply a mutation and notify subscribers (once per microtask). */
export function update(mutate: (s: State) => void = () => {}): void {
  mutate(state);
  if (scheduled) return;
  scheduled = true;
  queueMicrotask(() => {
    scheduled = false;
    for (const fn of listeners) fn(state);
  });
}

// ---------- selectors ----------

export const activeDoc = (s: State = state): DocState | undefined => s.docs.find((d) => d.id === s.activeId);
export const docById = (id: number, s: State = state): DocState | undefined =>
  s.docs.find((d) => d.id === id);
export const activeComments = (s: State = state): Comment[] => activeDoc(s)?.comments ?? [];
export const drafts = (doc: DocState | undefined = activeDoc()): Comment[] =>
  doc?.comments.filter((c) => c.status === "draft") ?? [];

// ---------- server status ----------

function newDoc(status: DocStatus): DocState {
  return { ...status, comments: [], agentMessages: [], selectedId: null, agentSeen: false, view: null };
}

/** Merge the backend status: adds new tabs, drops closed ones, keeps client-side data. */
export function applyServerStatus(status: ServerStatus): void {
  update((s) => {
    s.port = status.port;
    s.serverError = status.serverError;
    s.docs = status.documents.map((server) => {
      const doc = s.docs.find((d) => d.id === server.id) ?? newDoc(server);
      Object.assign(doc, server);
      if (server.listeners > 0) doc.agentSeen = true;
      return doc;
    });
    if (s.activeId !== null && !docById(s.activeId, s)) s.activeId = s.docs.at(-1)?.id ?? null;
    if (s.activeId === null && s.docs.length) s.activeId = s.docs[0]?.id ?? null;
  });
}

// ---------- actions ----------

let nextSeq = 1;
export const nextCommentId = (): string => `c${nextSeq++}`;

export function activate(id: number | null): void {
  update((s) => {
    if (id === null || docById(id, s)) s.activeId = id;
  });
}

/** Remove a closed tab and pick a neighbour. */
export function removeDoc(id: number): void {
  update((s) => {
    const idx = s.docs.findIndex((d) => d.id === id);
    if (idx < 0) return;
    s.docs.splice(idx, 1);
    if (s.activeId === id) s.activeId = (s.docs[idx] ?? s.docs[idx - 1])?.id ?? null;
  });
}

export function addComment(c: Comment, doc: DocState | undefined = activeDoc()): void {
  if (!doc) return;
  update(() => doc.comments.push(c));
}

export function removeComment(id: string): void {
  update((s) => {
    for (const doc of s.docs) {
      doc.comments = doc.comments.filter((c) => c.id !== id);
      if (doc.selectedId === id) doc.selectedId = null;
    }
  });
}

const ORDER: CommentStatus[] = ["draft", "sent", "delivered", "resolved"];

/** Move comments to `to` — only forward (draft → sent → delivered → resolved). */
export function setStatus(docId: number, ids: readonly string[], to: CommentStatus): void {
  update((s) => {
    for (const c of docById(docId, s)?.comments ?? []) {
      if (ids.includes(c.id) && ORDER.indexOf(to) > ORDER.indexOf(c.status)) c.status = to;
    }
  });
}

/**
 * Put comments back into the draft state so they can be sent again (manual
 * recovery if a delivery got lost on the way to the agent).
 */
export function resetToDraft(docId: number, ids: readonly string[]): void {
  update((s) => {
    for (const c of docById(docId, s)?.comments ?? []) {
      if (ids.includes(c.id) && c.status !== "resolved") c.status = "draft";
    }
  });
}

export function markAgentSeen(docId: number): void {
  update((s) => {
    const doc = docById(docId, s);
    if (doc) doc.agentSeen = true;
  });
}

export function addAgentMessage(docId: number, message: string, resolved: string[]): void {
  update((s) => {
    const doc = docById(docId, s);
    if (!doc) return;
    doc.agentSeen = true;
    doc.agentMessages.push({ message, resolved, createdAt: Date.now() });
  });
  setStatus(docId, resolved, "resolved");
}

export function selectComment(id: string | null): void {
  update((s) => {
    const doc = activeDoc(s);
    if (doc) doc.selectedId = id;
  });
}
