import { defineConfig } from "vite";
import solidPlugin from "vite-plugin-solid";

export default defineConfig({
  plugins: [solidPlugin()],
  resolve: {
    conditions: ["browser"],
  },
  server: {
    port: 1420,
    strictPort: true,
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
    server: {
      deps: {
        inline: ["solid-js", "@solidjs/testing-library"],
      },
    },
  },
});
