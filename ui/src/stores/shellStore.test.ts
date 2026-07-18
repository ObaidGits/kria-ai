import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { shellStore } from "./shellStore";
import { eventBus } from "./eventBus";

describe("shellStore", () => {
  beforeEach(() => {
    // Reset to defaults
    shellStore.setActiveSpace("converse");
    shellStore.setTheme("dark");
    shellStore.setDensity("focused");
    shellStore.setWindowMode("standard");
    shellStore.setPaletteOpen(false);
    shellStore.setInspectorTarget(null);
  });

  afterEach(() => {
    eventBus.clear();
  });

  describe("activeSpace", () => {
    it("defaults to converse", () => {
      expect(shellStore.activeSpace()).toBe("converse");
    });

    it("changes to a new Space and emits event", () => {
      const handler = vi.fn();
      eventBus.on("shell:space-changed", handler, "none");

      shellStore.setActiveSpace("memory");

      expect(shellStore.activeSpace()).toBe("memory");
      expect(handler).toHaveBeenCalledWith({ space: "memory", previous: "converse" });
    });

    it("does not emit when setting the same Space", () => {
      const handler = vi.fn();
      eventBus.on("shell:space-changed", handler, "none");

      shellStore.setActiveSpace("converse");
      expect(handler).not.toHaveBeenCalled();
    });
  });

  describe("windowMode", () => {
    it("defaults to standard", () => {
      expect(shellStore.windowMode()).toBe("standard");
    });

    it("changes mode and emits event", () => {
      const handler = vi.fn();
      eventBus.on("shell:mode-changed", handler, "none");

      shellStore.setWindowMode("immersive");

      expect(shellStore.windowMode()).toBe("immersive");
      expect(handler).toHaveBeenCalledWith({ mode: "immersive", previous: "standard" });
    });

    it("does not emit when setting the same mode", () => {
      const handler = vi.fn();
      eventBus.on("shell:mode-changed", handler, "none");

      shellStore.setWindowMode("standard");
      expect(handler).not.toHaveBeenCalled();
    });
  });

  describe("palette", () => {
    it("defaults to closed", () => {
      expect(shellStore.paletteOpen()).toBe(false);
    });

    it("opens and emits", () => {
      const handler = vi.fn();
      eventBus.on("shell:palette-toggled", handler, "none");

      shellStore.setPaletteOpen(true);

      expect(shellStore.paletteOpen()).toBe(true);
      expect(handler).toHaveBeenCalledWith({ open: true });
    });

    it("togglePalette flips the state", () => {
      shellStore.togglePalette();
      expect(shellStore.paletteOpen()).toBe(true);
      shellStore.togglePalette();
      expect(shellStore.paletteOpen()).toBe(false);
    });
  });

  describe("theme", () => {
    it("defaults to dark", () => {
      expect(shellStore.theme()).toBe("dark");
    });

    it("toggleTheme switches dark↔light", () => {
      shellStore.toggleTheme();
      expect(shellStore.theme()).toBe("light");
      shellStore.toggleTheme();
      expect(shellStore.theme()).toBe("dark");
    });

    it("emits theme-changed event", () => {
      const handler = vi.fn();
      eventBus.on("shell:theme-changed", handler, "none");

      shellStore.setTheme("light");
      expect(handler).toHaveBeenCalledWith({ theme: "light" });
    });
  });

  describe("density", () => {
    it("defaults to focused", () => {
      expect(shellStore.density()).toBe("focused");
    });

    it("accepts calm, focused, dense", () => {
      shellStore.setDensity("calm");
      expect(shellStore.density()).toBe("calm");
      shellStore.setDensity("dense");
      expect(shellStore.density()).toBe("dense");
    });
  });

  describe("inspectorTarget", () => {
    it("defaults to null", () => {
      expect(shellStore.inspectorTarget()).toBeNull();
    });

    it("sets and clears inspector target", () => {
      shellStore.setInspectorTarget({ type: "memory", id: "fact-1" });
      expect(shellStore.inspectorTarget()).toEqual({ type: "memory", id: "fact-1" });

      shellStore.setInspectorTarget(null);
      expect(shellStore.inspectorTarget()).toBeNull();
    });
  });
});
