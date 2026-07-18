import { describe, it, expect, beforeEach } from "vitest";
import { searchItems, groupResults, flattenGroups } from "./search";
import { recordUse, clearRecents } from "./recents";
import type { PaletteItem } from "./types";

function item(id: string, type: PaletteItem["type"], title: string, extra?: Partial<PaletteItem>): PaletteItem {
  return { id, type, title, run: () => {}, ...extra };
}

const ITEMS: PaletteItem[] = [
  item("space:settings", "space", "Settings"),
  item("space:memory", "space", "Memory"),
  item("cmd.theme", "command", "Toggle theme"),
  item("setting:volume", "setting", "Volume", { subtitle: "Voice · output" }),
  item("thread:1", "thread", "Trip planning"),
];

describe("searchItems", () => {
  beforeEach(() => clearRecents());

  it("returns all items for an empty query", () => {
    expect(searchItems(ITEMS, "").length).toBe(ITEMS.length);
  });

  it("filters to fuzzy matches for a non-empty query", () => {
    const r = searchItems(ITEMS, "sett");
    expect(r.some((x) => x.item.id === "space:settings")).toBe(true);
    expect(r.some((x) => x.item.id === "space:memory")).toBe(false);
  });

  it("matches against the subtitle as well as the title", () => {
    const r = searchItems(ITEMS, "voice");
    expect(r.some((x) => x.item.id === "setting:volume")).toBe(true);
  });

  it("orders stronger text matches first", () => {
    const r = searchItems(ITEMS, "the");
    expect(r[0].item.id).toBe("cmd.theme");
  });

  it("applies a bounded recent-use promotion on ties", () => {
    const baseline = searchItems(ITEMS, "").map((result) => result.item.id);
    const originalIndex = baseline.indexOf("thread:1");
    recordUse("thread:1");
    const ranked = searchItems(ITEMS, "").map((result) => result.item.id);
    const promotedIndex = ranked.indexOf("thread:1");
    expect(promotedIndex).toBeLessThan(originalIndex);
    expect(originalIndex - promotedIndex).toBeLessThanOrEqual(2);
    expect([...ranked].sort()).toEqual([...baseline].sort());
  });

  it("is deterministic for equal scores (alphabetical tie-break)", () => {
    const a = searchItems(ITEMS, "");
    const b = searchItems(ITEMS, "");
    expect(a.map((x) => x.item.id)).toEqual(b.map((x) => x.item.id));
  });
});

describe("groupResults", () => {
  beforeEach(() => clearRecents());

  it("groups results by type in canonical order (commands before spaces)", () => {
    const groups = groupResults(searchItems(ITEMS, ""));
    const types = groups.map((g) => g.type);
    expect(types.indexOf("command")).toBeLessThan(types.indexOf("space"));
  });

  it("labels each group", () => {
    const groups = groupResults(searchItems(ITEMS, "sett"));
    expect(groups[0].label).toBeTruthy();
  });

  it("flattening preserves group ordering", () => {
    const groups = groupResults(searchItems(ITEMS, ""));
    const flat = flattenGroups(groups);
    expect(flat.length).toBe(ITEMS.length);
    // First flat item belongs to the first group.
    expect(flat[0].item.type).toBe(groups[0].type);
  });
});
