// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

const submitFeedback = vi.fn<(doc: number, items: unknown[]) => Promise<void>>(async () => {});
vi.mock("./backend", () => ({
  backend: { submitFeedback: (doc: number, items: unknown[]) => submitFeedback(doc, items) },
}));
vi.mock("./ui/toast", () => ({ toast: vi.fn() }));

const { commitDraft, resendComment, sendDrafts } = await import("./actions");
const { activate, setStatus, update } = await import("./store");
const { doc, resetStore } = await import("./test-utils");

/** `[doc, ids]` of the n-th `submitFeedback` call. */
const sent = (call: number) => {
  const args = submitFeedback.mock.calls[call];
  return args && [args[0], (args[1] as { id: string }[]).map((i) => i.id)];
};

describe("actions", () => {
  beforeEach(() => {
    submitFeedback.mockClear();
    resetStore(1, 2);
  });

  it("collects drafts when instant send is off, then sends them together", async () => {
    await commitDraft({ id: "c1", kind: "general", page: 0 }, "one");
    await commitDraft({ id: "c2", kind: "general", page: 0 }, "two");
    expect(submitFeedback).not.toHaveBeenCalled();
    await sendDrafts();
    expect(sent(0)).toEqual([1, ["c1", "c2"]]);
    expect(doc(1).comments.every((c) => c.status === "sent")).toBe(true);
  });

  it("sends drafts of every tab to its own document", async () => {
    await commitDraft({ id: "c1", kind: "general", page: 0 }, "one");
    activate(2);
    await commitDraft({ id: "c2", kind: "general", page: 0 }, "two");
    await sendDrafts();
    expect([sent(0), sent(1)]).toEqual([
      [1, ["c1"]],
      [2, ["c2"]],
    ]);
  });

  it("never sends an area comment before its screenshot is ready", async () => {
    update((s) => {
      s.live = true;
    });
    let resolveImage: (shot: { image: string; preview: string }) => void = () => {};
    const imagePromise = new Promise<{ image: string; preview: string }>((r) => {
      resolveImage = r;
    });
    const pending = commitDraft(
      { id: "c3", kind: "area", page: 1, rect: [0, 0, 1, 1], imagePromise },
      "bigger",
    );
    // another comment is sent in the meantime
    await commitDraft({ id: "c4", kind: "general", page: 0 }, "other");
    expect(sent(0)).toEqual([1, ["c4"]]);

    resolveImage({ image: "iVBOR", preview: "small" });
    await pending;
    const second = submitFeedback.mock.calls[1]?.[1] as { id: string; image: string }[];
    expect(second).toEqual([expect.objectContaining({ id: "c3", image: "iVBOR", preview: "small" })]);
  });

  it("keeps drafts when sending fails", async () => {
    submitFeedback.mockRejectedValueOnce(new Error("offline"));
    await commitDraft({ id: "c5", kind: "general", page: 0 }, "x");
    await sendDrafts();
    expect(doc(1).comments[0]?.status).toBe("draft");
  });

  it("sends a delivered comment again on request", async () => {
    await commitDraft({ id: "c6", kind: "general", page: 0 }, "x");
    await sendDrafts();
    setStatus(1, ["c6"], "delivered");
    await resendComment(doc(1), "c6");
    expect(sent(1)).toEqual([1, ["c6"]]);
    expect(doc(1).comments[0]?.status).toBe("sent");
  });
});
