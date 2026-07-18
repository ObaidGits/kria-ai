import { defineConfig } from "vite";
import solidPlugin from "vite-plugin-solid";

export default defineConfig({
  plugins: [solidPlugin()],
  resolve: {
    conditions: ["development", "browser"],
    // Keep Kobalte, Solid primitives, app code, and test renderer on one owner
    // runtime. Duplicate Solid modules create ownerless computations in tests
    // and can leak reactive work in production bundles.
    dedupe: ["solid-js", "solid-js/web", "solid-js/store"],
  },
  server: {
    port: 1420,
    strictPort: true,
    warmup: {
      clientFiles: ["./src/shell/spaces/MemorySpace.tsx"],
    },
  },
  build: {
    target: "esnext",
    outDir: "dist",
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (!id.includes("node_modules")) return undefined;
          if (id.includes("solid-js")) return "vendor-solid";
          if (id.includes("@tauri-apps")) return "vendor-tauri";
          if (id.includes("marked") || id.includes("dompurify") || id.includes("highlight.js")) {
            return "vendor-markdown";
          }
          if (id.includes("chart.js")) return "vendor-charts";
          return "vendor";
        },
      },
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: "./src/test/setup.ts",
    include: ["src/**/*.test.{ts,tsx}"],
    css: {
      include: /.+/,
    },
    server: {
      deps: {
        inline: [
          "solid-js",
          "@solidjs/testing-library",
          /@kobalte\//,
          /@solid-primitives\//,
          /@corvu\//,
          /@tanstack\/solid-/,
          "@tanstack/solid-virtual",
          "@tanstack/virtual-core",
          "solid-presence",
          "solid-prevent-scroll",
        ],
      },
    },
  },
});
