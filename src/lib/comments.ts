/** Comment model and the rules that turn user input into agent instructions. */
import { t } from "../i18n";
import type { NRect } from "./geometry";

export type CommentKind = "text" | "area" | "general";
export type CommentStatus = "draft" | "sent" | "delivered" | "resolved";
export type ComposerMode = "comment" | "replace" | "delete";

export interface Comment {
  /** Stable id shared with the agent, e.g. `c3`. */
  id: string;
  kind: CommentKind;
  /** 1-based page, 0 for general comments. */
  page: number;
  quote?: string;
  comment: string;
  /** Area selection. */
  rect?: NRect;
  /** Line boxes of a text selection. */
  rects?: NRect[];
  /** PNG screenshot, base64 without prefix (full resolution). */
  image?: string;
  /** Downscaled screenshot sent to the agent by default (saves tokens). */
  preview?: string;
  /** Page text around a text selection (helps the agent find the spot). */
  context?: string;
  areaText?: string;
  status: CommentStatus;
  createdAt: number;
}

/** A comment that is being composed (not yet committed). */
export type CommentDraft = Omit<Comment, "comment" | "status" | "createdAt"> & {
  imagePromise?: Promise<Screenshot>;
};

/** A rendered area: full resolution plus a small preview. */
export interface Screenshot {
  image: string;
  preview: string;
}

export interface AgentMessage {
  message: string;
  resolved: string[];
  createdAt: number;
}

/** Wire format expected by the backend (`FeedbackItem` in Rust). */
export interface FeedbackPayload {
  id: string;
  kind: CommentKind;
  page: number;
  quote: string | null;
  comment: string;
  rect: NRect | null;
  image: string | null;
  preview: string | null;
  areaText: string | null;
  context: string | null;
}

export function toPayload(c: Comment): FeedbackPayload {
  return {
    id: c.id,
    kind: c.kind,
    page: c.page,
    quote: c.quote ?? null,
    comment: c.comment,
    rect: c.rect ?? null,
    image: c.image ?? null,
    preview: c.preview ?? null,
    areaText: c.areaText ?? null,
    context: c.context ?? null,
  };
}

/** Collapse whitespace of a text selection while keeping line breaks. */
export function normalizeQuote(raw: string): string {
  return raw
    .replace(/[ \t]+/g, " ")
    .replace(/\s*\n\s*/g, "\n")
    .trim();
}

/**
 * Build the instruction sent to the agent for a composer submission.
 * Returns `null` when there is nothing to send (replace with unchanged text).
 */
export function composeInstruction(
  mode: ComposerMode,
  kind: CommentKind,
  input: string,
  quote = "",
): string | null {
  const text = input.trim();
  const remove = t(kind === "area" ? "comment.deleteArea" : "comment.deleteText");
  switch (mode) {
    case "comment":
      return text;
    case "delete":
      return text ? `${remove} ${text}` : remove;
    case "replace":
      if (!text) return remove;
      return text === quote.trim() ? null : t("comment.replaceWith", { text });
  }
}

/** Number shown on pins and cards (`c12` → `12`). */
export const displayNumber = (id: string): string => id.replace(/^c/, "");
