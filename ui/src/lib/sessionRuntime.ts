//! Per-session chat runtime state (Issue-2 fix).
//!
//! Pure, framework-agnostic reducers for the per-session message + thinking
//! buffers. Extracted from the store so the routing semantics are unit-testable
//! without a browser: switching chats must never lose an in-flight generation,
//! and streamed tokens must accumulate in the OWNING session's bucket.

export interface RuntimeMessageLike {
  role: string;
  content: string;
}

export interface SessionRuntimeState<M> {
  messages: M[];
  thinking: boolean;
}

export type SessionBuckets<M> = Record<string, SessionRuntimeState<M>>;

function emptyState<M>(): SessionRuntimeState<M> {
  return { messages: [], thinking: false };
}

/** Messages for a session bucket (empty when the bucket does not exist yet). */
export function bucketMessages<M>(buckets: SessionBuckets<M>, key: string): M[] {
  return buckets[key]?.messages ?? [];
}

/** Thinking flag for a session bucket (false when absent). */
export function bucketThinking<M>(buckets: SessionBuckets<M>, key: string): boolean {
  return buckets[key]?.thinking ?? false;
}

/**
 * Immutably update one session's message list. Other sessions are untouched —
 * this isolation is what lets a background chat keep streaming while another is
 * focused.
 */
export function updateBucketMessages<M>(
  buckets: SessionBuckets<M>,
  key: string,
  updater: (prev: M[]) => M[]
): SessionBuckets<M> {
  const cur = buckets[key] ?? emptyState<M>();
  return { ...buckets, [key]: { ...cur, messages: updater(cur.messages) } };
}

/** Immutably update one session's thinking flag. */
export function updateBucketThinking<M>(
  buckets: SessionBuckets<M>,
  key: string,
  value: boolean | ((prev: boolean) => boolean)
): SessionBuckets<M> {
  const cur = buckets[key] ?? emptyState<M>();
  const next = typeof value === "function" ? value(cur.thinking) : value;
  return { ...buckets, [key]: { ...cur, thinking: next } };
}

/**
 * Append a streamed token to the assistant message at the tail of a session's
 * transcript, creating a fresh assistant message when the tail is not one.
 */
export function appendAssistantToken<M extends RuntimeMessageLike>(
  messages: M[],
  text: string,
  makeAssistant: (text: string) => M
): M[] {
  const last = messages[messages.length - 1];
  if (last && last.role === "assistant") {
    return [...messages.slice(0, -1), { ...last, content: last.content + text } as M];
  }
  return [...messages, makeAssistant(text)];
}
