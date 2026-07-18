import "@testing-library/jest-dom/vitest";
import { cleanup } from "@solidjs/testing-library";
import { afterEach } from "vitest";

// jsdom lacks ResizeObserver / IntersectionObserver, which several Kobalte
// primitives (Tabs indicator, popovers) construct on mount. Provide inert
// polyfills so component tests can render them.
if (typeof globalThis.ResizeObserver === "undefined") {
	globalThis.ResizeObserver = class {
		observe() {}
		unobserve() {}
		disconnect() {}
	} as unknown as typeof ResizeObserver;
}

if (typeof globalThis.IntersectionObserver === "undefined") {
	globalThis.IntersectionObserver = class {
		root = null;
		rootMargin = "";
		thresholds = [];
		observe() {}
		unobserve() {}
		disconnect() {}
		takeRecords() {
			return [];
		}
	} as unknown as typeof IntersectionObserver;
}

// jsdom declares canvas APIs but logs a not-implemented error on every call.
// A null context accurately models this test environment's lack of WebGL.
if (typeof globalThis.HTMLCanvasElement !== "undefined") {
	Object.defineProperty(globalThis.HTMLCanvasElement.prototype, "getContext", {
		configurable: true,
		value: () => null,
	});
}

// jsdom exposes scrollTo but routes calls to its noisy not-implemented stub.
// Overlay cleanup restores scroll position, so provide the browser-equivalent
// no-op needed by component tests without a layout/scrolling engine.
if (typeof globalThis.window !== "undefined") {
	Object.defineProperty(globalThis.window, "scrollTo", {
		configurable: true,
		value: () => undefined,
	});
}

afterEach(() => {
	cleanup();
});
