import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  activate,
  activeComments,
  addAgentMessage,
  addComment,
  applyServerStatus,
  drafts,
  nextCommentId,
  removeComment,
  removeDoc,
  selectComment,
  setStatus,
  state,
  subscribe,
  update,
} from "./store";
import { comment, doc, docStatus, resetStore, serverStatus } from "./test-utils";

describe("store", () => {
  beforeEach(() => resetStore(1, 2));

  it("generates unique ids", () => {
    const a = nextCommentId();
    expect(a).not.toBe(nextCommentId());
    expect(a).toMatch(/^c\d+$/);
  });

  it("keeps comments per tab", () => {
    addComment(comment("a"));
    activate(2);
    addComment(comment("b"));
    expect(activeComments().map((c) => c.id)).toEqual(["b"]);
    activate(1);
    expect(activeComments().map((c) => c.id)).toEqual(["a"]);
  });

  it("only moves comment status forward", () => {
    addComment(comment("a"));
    setStatus(1, ["a"], "delivered");
    setStatus(1, ["a"], "sent"); // late "sent" must not undo "delivered"
    expect(doc(1).comments[0]?.status).toBe("delivered");
  });

  it("resolves comments from agent replies of the right document", () => {
    addComment(comment("a", "delivered"));
    addComment(comment("b", "delivered"), doc(2));
    addAgentMessage(2, "done", ["b"]);
    expect(doc(1).comments[0]?.status).toBe("delivered");
    expect(doc(2).comments[0]?.status).toBe("resolved");
    expect(doc(2).agentSeen).toBe(true);
    expect(doc(1).agentMessages).toHaveLength(0);
  });

  it("clears the selection when the selected comment is removed", () => {
    addComment(comment("a"));
    selectComment("a");
    removeComment("a");
    expect(doc(1).selectedId).toBeNull();
    expect(doc(1).comments).toHaveLength(0);
  });

  it("lists drafts of a document", () => {
    addComment(comment("a"));
    addComment(comment("b", "sent"));
    expect(drafts().map((c) => c.id)).toEqual(["a"]);
  });

  it("merges server status without losing client data", () => {
    addComment(comment("a"));
    applyServerStatus(serverStatus([docStatus(1, { pending: 2, listeners: 1 }), docStatus(3)]));
    expect(state.docs.map((d) => d.id)).toEqual([1, 3]);
    expect(doc(1).comments).toHaveLength(1);
    expect(doc(1).pending).toBe(2);
    expect(doc(1).agentSeen).toBe(true);
  });

  it("activates a neighbour when the active tab is closed", () => {
    applyServerStatus(serverStatus([docStatus(1), docStatus(2), docStatus(3)]));
    activate(2);
    removeDoc(2);
    expect(state.activeId).toBe(3);
    removeDoc(3);
    expect(state.activeId).toBe(1);
    removeDoc(1);
    expect(state.activeId).toBeNull();
  });

  it("falls back to an existing tab when the active one disappears from the status", () => {
    activate(2);
    applyServerStatus(serverStatus([docStatus(1)]));
    expect(state.activeId).toBe(1);
  });

  it("batches notifications per microtask", async () => {
    await Promise.resolve();
    const fn = vi.fn();
    const off = subscribe(fn);
    update();
    update();
    addComment(comment("x"));
    expect(fn).not.toHaveBeenCalled();
    await Promise.resolve();
    expect(fn).toHaveBeenCalledTimes(1);
    off();
  });
});
