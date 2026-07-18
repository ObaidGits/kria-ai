import { describe, it, expect } from "vitest";
import {
  appendAssistantToken,
  bucketMessages,
  bucketThinking,
  updateBucketMessages,
  updateBucketThinking,
  type SessionBuckets,
} from "./sessionRuntime";

interface Msg {
  id: string;
  role: string;
  content: string;
}

const mkAssistant = (text: string): Msg => ({
  id: `a-${Math.random()}`,
  role: "assistant",
  content: text,
});

describe("appendAssistantToken", () => {
  it("creates an assistant message when the tail is not assistant", () => {
    const out = appendAssistantToken<Msg>(
      [{ id: "u1", role: "user", content: "hi" }],
      "Hel",
      mkAssistant
    );
    expect(out).toHaveLength(2);
    expect(out[1]).toMatchObject({ role: "assistant", content: "Hel" });
  });

  it("appends to the trailing assistant message", () => {
    const out = appendAssistantToken<Msg>(
      [{ id: "a1", role: "assistant", content: "Hel" }],
      "lo",
      mkAssistant
    );
    expect(out).toHaveLength(1);
    expect(out[0].content).toBe("Hello");
  });
});

describe("per-session isolation (Issue-2 root-cause fix)", () => {
  it("writing one session never mutates another", () => {
    let buckets: SessionBuckets<Msg> = {};
    buckets = updateBucketMessages(buckets, "s1", (p) =>
      appendAssistantToken(p, "A", mkAssistant)
    );
    buckets = updateBucketMessages(buckets, "s2", (p) =>
      appendAssistantToken(p, "B", mkAssistant)
    );

    expect(bucketMessages(buckets, "s1")[0].content).toBe("A");
    expect(bucketMessages(buckets, "s2")[0].content).toBe("B");
  });

  it("two chats stream concurrently and each accumulates its own response", () => {
    // Simulates: Chat1 generating, switch to Chat2, Chat2 generating, tokens for
    // BOTH keep arriving interleaved. Neither is lost (the original bug).
    let buckets: SessionBuckets<Msg> = {};
    const feed = (sid: string, text: string) => {
      buckets = updateBucketMessages(buckets, sid, (p) =>
        appendAssistantToken(p, text, mkAssistant)
      );
    };

    feed("chat1", "Hello ");
    feed("chat2", "World ");
    feed("chat1", "from one");
    feed("chat2", "from two");

    expect(bucketMessages(buckets, "chat1")[0].content).toBe("Hello from one");
    expect(bucketMessages(buckets, "chat2")[0].content).toBe("World from two");
  });

  it("returning to a background chat restores its in-progress transcript", () => {
    let buckets: SessionBuckets<Msg> = {};
    // Chat1 starts generating.
    buckets = updateBucketThinking(buckets, "chat1", true);
    buckets = updateBucketMessages(buckets, "chat1", (p) =>
      appendAssistantToken(p, "partial answer", mkAssistant)
    );
    // User switches to Chat2 (active key changes — no state cleared).
    buckets = updateBucketThinking(buckets, "chat2", true);
    buckets = updateBucketMessages(buckets, "chat2", (p) =>
      appendAssistantToken(p, "chat2 answer", mkAssistant)
    );

    // Returning to Chat1: its transcript + spinner are intact.
    expect(bucketMessages(buckets, "chat1")[0].content).toBe("partial answer");
    expect(bucketThinking(buckets, "chat1")).toBe(true);
    // Chat2 unaffected.
    expect(bucketMessages(buckets, "chat2")[0].content).toBe("chat2 answer");
    expect(bucketThinking(buckets, "chat2")).toBe(true);
  });

  it("thinking is per-session: finishing one does not clear the other", () => {
    let buckets: SessionBuckets<Msg> = {};
    buckets = updateBucketThinking(buckets, "chat1", true);
    buckets = updateBucketThinking(buckets, "chat2", true);
    // Chat1 done.
    buckets = updateBucketThinking(buckets, "chat1", false);

    expect(bucketThinking(buckets, "chat1")).toBe(false);
    expect(bucketThinking(buckets, "chat2")).toBe(true);
  });

  it("missing bucket reads are safe defaults", () => {
    const buckets: SessionBuckets<Msg> = {};
    expect(bucketMessages(buckets, "nope")).toEqual([]);
    expect(bucketThinking(buckets, "nope")).toBe(false);
  });
});
