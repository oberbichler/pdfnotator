/** User actions that talk to the backend. */
import { backend } from "./backend";
import { t } from "./i18n";
import type { Comment, CommentDraft } from "./lib/comments";
import { toPayload } from "./lib/comments";
import {
  activeDoc,
  addComment,
  type DocState,
  drafts,
  resetToDraft,
  setStatus,
  state,
  update,
} from "./store";
import { toast } from "./ui/toast";

/** Comments whose screenshot is still being rendered — never sent without it. */
const capturing = new Set<string>();

/** Drafts being submitted right now (`doc:comment`), so overlapping sends
 * (e.g. Cmd+Enter reaching both the composer and the global shortcut) don't
 * deliver a comment twice. */
const sending = new Set<string>();

/** Send the drafts of one document (default: all documents) to the agent queue. */
export async function sendDrafts(only?: DocState): Promise<void> {
  for (const doc of only ? [only] : state.docs) {
    const key = (id: string) => `${doc.id}:${id}`;
    const ready = drafts(doc).filter((c) => !capturing.has(c.id) && !sending.has(key(c.id)));
    if (!ready.length) continue;
    const ids = ready.map((c) => c.id);
    for (const id of ids) sending.add(key(id));
    try {
      await backend.submitFeedback(doc.id, ready.map(toPayload));
      setStatus(doc.id, ids, "sent");
    } catch (error) {
      toast(t("error.send", { error: String(error) }), "error");
    } finally {
      for (const id of ids) sending.delete(key(id));
    }
  }
}

/** Turn a composed draft into a comment of the active document (sent right away in live mode). */
export async function commitDraft(draft: CommentDraft, text: string): Promise<void> {
  const doc = activeDoc();
  if (!doc) return;
  const { imagePromise, ...rest } = draft;
  const comment: Comment = { ...rest, comment: text, status: "draft", createdAt: Date.now() };
  addComment(comment, doc);
  if (imagePromise) {
    capturing.add(comment.id);
    try {
      const shot = await imagePromise;
      comment.image = shot.image;
      comment.preview = shot.preview;
      comment.clean = shot.clean;
    } catch (error) {
      toast(t("error.screenshot", { error: String(error) }), "error");
    } finally {
      capturing.delete(comment.id);
      update();
    }
  }
  if (state.live) await sendDrafts(doc);
}

/** Send a comment again (it may never have reached the agent). */
export async function resendComment(doc: DocState, id: string): Promise<void> {
  resetToDraft(doc.id, [id]);
  await sendDrafts(doc);
}

/** Open a `.typ`/`.pdf` file in a tab. */
export async function openDocument(path: string): Promise<void> {
  try {
    await backend.openDocument(path);
  } catch (error) {
    toast(String(error), "error");
  }
}

/** Close a tab (releases an agent waiting on it). */
export async function closeDocument(id: number): Promise<void> {
  try {
    await backend.closeDocument(id);
  } catch (error) {
    toast(String(error), "error");
  }
}
