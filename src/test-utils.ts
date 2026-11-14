/** Helpers shared by tests (not bundled: only imported from *.test.ts). */
import type { DocStatus, ServerStatus } from "./backend";
import type { Comment } from "./lib/comments";
import { applyServerStatus, state, update } from "./store";

export function docStatus(id: number, patch: Partial<DocStatus> = {}): DocStatus {
  return {
    id,
    path: `/docs/doc${id}.pdf`,
    source: null,
    compileError: null,
    version: 1,
    listeners: 0,
    pending: 0,
    currentPage: 1,
    pageCount: 3,
    ...patch,
  };
}

export function serverStatus(documents: DocStatus[], patch: Partial<ServerStatus> = {}): ServerStatus {
  return { port: 4747, serverError: null, documents, ...patch };
}

/** Reset the store to the given open documents (first one active). */
export function resetStore(...ids: number[]): void {
  update((s) => {
    s.docs = [];
    s.activeId = null;
    s.live = false;
    s.serverError = null;
  });
  applyServerStatus(serverStatus(ids.map((id) => docStatus(id))));
  update((s) => {
    s.activeId = ids[0] ?? null;
  });
}

export const comment = (id: string, status: Comment["status"] = "draft"): Comment => ({
  id,
  kind: "general",
  page: 0,
  comment: id,
  status,
  createdAt: Date.now(),
});

export const doc = (id: number) => {
  const d = state.docs.find((x) => x.id === id);
  if (!d) throw new Error(`doc ${id} missing`);
  return d;
};
