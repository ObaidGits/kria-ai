import { describe, expect, it, vi } from "vitest";

const { invokeMock, listenMock } = vi.hoisted(() => {
  const invokeMock = vi.fn(async () => null);
  const listenMock = vi.fn(async () => () => {});
  vi.stubGlobal("setInterval", vi.fn(() => 1));
  return { invokeMock, listenMock };
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/api/event", () => ({ listen: listenMock }));
vi.mock("highlight.js/styles/github-dark.css?url", () => ({ default: "dark.css" }));
vi.mock("highlight.js/styles/github.css?url", () => ({ default: "light.css" }));

import { groupSessionsByRecency, type Session } from "./app";

const DAY = 86_400_000;

function mk(id: string, updatedAt: number, extra: Partial<Session> = {}): Session {
  return { id, title: id, updatedAt, ...extra };
}

describe("groupSessionsByRecency", () => {
  // Fixed "now" at midday so day-boundary math is unambiguous.
  const now = new Date("2026-06-16T12:00:00").getTime();
  const todayStart = (() => {
    const d = new Date(now);
    d.setHours(0, 0, 0, 0);
    return d.getTime();
  })();

  it("buckets sessions by recency", () => {
    const sessions = [
      mk("today", todayStart + 1000),
      mk("yesterday", todayStart - 1000),
      mk("prev7", todayStart - 3 * DAY),
      mk("older", todayStart - 30 * DAY),
    ];
    const g = groupSessionsByRecency(sessions, now);
    expect(g.today.map((s) => s.id)).toEqual(["today"]);
    expect(g.yesterday.map((s) => s.id)).toEqual(["yesterday"]);
    expect(g.previous7Days.map((s) => s.id)).toEqual(["prev7"]);
    expect(g.older.map((s) => s.id)).toEqual(["older"]);
  });

  it("floats pinned (non-archived) into the pinned group regardless of age", () => {
    const sessions = [
      mk("old-but-pinned", todayStart - 100 * DAY, { pinned: true }),
      mk("today", todayStart + 1000),
    ];
    const g = groupSessionsByRecency(sessions, now);
    expect(g.pinned.map((s) => s.id)).toEqual(["old-but-pinned"]);
    expect(g.older).toHaveLength(0);
    expect(g.today.map((s) => s.id)).toEqual(["today"]);
  });

  it("separates archived sessions and excludes them from time/pinned groups", () => {
    const sessions = [
      mk("archived", todayStart + 1000, { archived: true }),
      mk("archived-pinned", todayStart + 2000, { archived: true, pinned: true }),
      mk("today", todayStart + 3000),
    ];
    const g = groupSessionsByRecency(sessions, now);
    expect(g.archived.map((s) => s.id).sort()).toEqual(["archived", "archived-pinned"]);
    expect(g.pinned).toHaveLength(0);
    expect(g.today.map((s) => s.id)).toEqual(["today"]);
  });

  it("sorts each group newest-first", () => {
    const sessions = [
      mk("a", todayStart + 1000),
      mk("b", todayStart + 5000),
      mk("c", todayStart + 3000),
    ];
    const g = groupSessionsByRecency(sessions, now);
    expect(g.today.map((s) => s.id)).toEqual(["b", "c", "a"]);
  });
});
