// vite.config.ts
import { defineConfig } from "file:///media/obaid/SSD/KRIA/ui/node_modules/vite/dist/node/index.js";
import solidPlugin from "file:///media/obaid/SSD/KRIA/ui/node_modules/vite-plugin-solid/dist/esm/index.mjs";
var vite_config_default = defineConfig({
  plugins: [solidPlugin()],
  resolve: {
    conditions: ["development", "browser"],
    // Keep Kobalte, Solid primitives, app code, and test renderer on one owner
    // runtime. Duplicate Solid modules create ownerless computations in tests
    // and can leak reactive work in production bundles.
    dedupe: ["solid-js", "solid-js/web", "solid-js/store"]
  },
  server: {
    port: 1420,
    strictPort: true,
    warmup: {
      clientFiles: ["./src/shell/spaces/MemorySpace.tsx"]
    }
  },
  build: {
    target: "esnext",
    outDir: "dist",
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (!id.includes("node_modules")) return void 0;
          if (id.includes("solid-js")) return "vendor-solid";
          if (id.includes("@tauri-apps")) return "vendor-tauri";
          if (id.includes("marked") || id.includes("dompurify") || id.includes("highlight.js")) {
            return "vendor-markdown";
          }
          if (id.includes("chart.js")) return "vendor-charts";
          return "vendor";
        }
      }
    }
  },
  test: {
    environment: "jsdom",
    setupFiles: "./src/test/setup.ts",
    include: ["src/**/*.test.{ts,tsx}"],
    css: {
      include: /.+/
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
          "solid-prevent-scroll"
        ]
      }
    }
  }
});
export {
  vite_config_default as default
};
//# sourceMappingURL=data:application/json;base64,ewogICJ2ZXJzaW9uIjogMywKICAic291cmNlcyI6IFsidml0ZS5jb25maWcudHMiXSwKICAic291cmNlc0NvbnRlbnQiOiBbImNvbnN0IF9fdml0ZV9pbmplY3RlZF9vcmlnaW5hbF9kaXJuYW1lID0gXCIvbWVkaWEvb2JhaWQvU1NEL0tSSUEvdWlcIjtjb25zdCBfX3ZpdGVfaW5qZWN0ZWRfb3JpZ2luYWxfZmlsZW5hbWUgPSBcIi9tZWRpYS9vYmFpZC9TU0QvS1JJQS91aS92aXRlLmNvbmZpZy50c1wiO2NvbnN0IF9fdml0ZV9pbmplY3RlZF9vcmlnaW5hbF9pbXBvcnRfbWV0YV91cmwgPSBcImZpbGU6Ly8vbWVkaWEvb2JhaWQvU1NEL0tSSUEvdWkvdml0ZS5jb25maWcudHNcIjtpbXBvcnQgeyBkZWZpbmVDb25maWcgfSBmcm9tIFwidml0ZVwiO1xuaW1wb3J0IHNvbGlkUGx1Z2luIGZyb20gXCJ2aXRlLXBsdWdpbi1zb2xpZFwiO1xuXG5leHBvcnQgZGVmYXVsdCBkZWZpbmVDb25maWcoe1xuICBwbHVnaW5zOiBbc29saWRQbHVnaW4oKV0sXG4gIHJlc29sdmU6IHtcbiAgICBjb25kaXRpb25zOiBbXCJkZXZlbG9wbWVudFwiLCBcImJyb3dzZXJcIl0sXG4gICAgLy8gS2VlcCBLb2JhbHRlLCBTb2xpZCBwcmltaXRpdmVzLCBhcHAgY29kZSwgYW5kIHRlc3QgcmVuZGVyZXIgb24gb25lIG93bmVyXG4gICAgLy8gcnVudGltZS4gRHVwbGljYXRlIFNvbGlkIG1vZHVsZXMgY3JlYXRlIG93bmVybGVzcyBjb21wdXRhdGlvbnMgaW4gdGVzdHNcbiAgICAvLyBhbmQgY2FuIGxlYWsgcmVhY3RpdmUgd29yayBpbiBwcm9kdWN0aW9uIGJ1bmRsZXMuXG4gICAgZGVkdXBlOiBbXCJzb2xpZC1qc1wiLCBcInNvbGlkLWpzL3dlYlwiLCBcInNvbGlkLWpzL3N0b3JlXCJdLFxuICB9LFxuICBzZXJ2ZXI6IHtcbiAgICBwb3J0OiAxNDIwLFxuICAgIHN0cmljdFBvcnQ6IHRydWUsXG4gICAgd2FybXVwOiB7XG4gICAgICBjbGllbnRGaWxlczogW1wiLi9zcmMvc2hlbGwvc3BhY2VzL01lbW9yeVNwYWNlLnRzeFwiXSxcbiAgICB9LFxuICB9LFxuICBidWlsZDoge1xuICAgIHRhcmdldDogXCJlc25leHRcIixcbiAgICBvdXREaXI6IFwiZGlzdFwiLFxuICAgIHJvbGx1cE9wdGlvbnM6IHtcbiAgICAgIG91dHB1dDoge1xuICAgICAgICBtYW51YWxDaHVua3MoaWQpIHtcbiAgICAgICAgICBpZiAoIWlkLmluY2x1ZGVzKFwibm9kZV9tb2R1bGVzXCIpKSByZXR1cm4gdW5kZWZpbmVkO1xuICAgICAgICAgIGlmIChpZC5pbmNsdWRlcyhcInNvbGlkLWpzXCIpKSByZXR1cm4gXCJ2ZW5kb3Itc29saWRcIjtcbiAgICAgICAgICBpZiAoaWQuaW5jbHVkZXMoXCJAdGF1cmktYXBwc1wiKSkgcmV0dXJuIFwidmVuZG9yLXRhdXJpXCI7XG4gICAgICAgICAgaWYgKGlkLmluY2x1ZGVzKFwibWFya2VkXCIpIHx8IGlkLmluY2x1ZGVzKFwiZG9tcHVyaWZ5XCIpIHx8IGlkLmluY2x1ZGVzKFwiaGlnaGxpZ2h0LmpzXCIpKSB7XG4gICAgICAgICAgICByZXR1cm4gXCJ2ZW5kb3ItbWFya2Rvd25cIjtcbiAgICAgICAgICB9XG4gICAgICAgICAgaWYgKGlkLmluY2x1ZGVzKFwiY2hhcnQuanNcIikpIHJldHVybiBcInZlbmRvci1jaGFydHNcIjtcbiAgICAgICAgICByZXR1cm4gXCJ2ZW5kb3JcIjtcbiAgICAgICAgfSxcbiAgICAgIH0sXG4gICAgfSxcbiAgfSxcbiAgdGVzdDoge1xuICAgIGVudmlyb25tZW50OiBcImpzZG9tXCIsXG4gICAgc2V0dXBGaWxlczogXCIuL3NyYy90ZXN0L3NldHVwLnRzXCIsXG4gICAgaW5jbHVkZTogW1wic3JjLyoqLyoudGVzdC57dHMsdHN4fVwiXSxcbiAgICBjc3M6IHtcbiAgICAgIGluY2x1ZGU6IC8uKy8sXG4gICAgfSxcbiAgICBzZXJ2ZXI6IHtcbiAgICAgIGRlcHM6IHtcbiAgICAgICAgaW5saW5lOiBbXG4gICAgICAgICAgXCJzb2xpZC1qc1wiLFxuICAgICAgICAgIFwiQHNvbGlkanMvdGVzdGluZy1saWJyYXJ5XCIsXG4gICAgICAgICAgL0Brb2JhbHRlXFwvLyxcbiAgICAgICAgICAvQHNvbGlkLXByaW1pdGl2ZXNcXC8vLFxuICAgICAgICAgIC9AY29ydnVcXC8vLFxuICAgICAgICAgIC9AdGFuc3RhY2tcXC9zb2xpZC0vLFxuICAgICAgICAgIFwiQHRhbnN0YWNrL3NvbGlkLXZpcnR1YWxcIixcbiAgICAgICAgICBcIkB0YW5zdGFjay92aXJ0dWFsLWNvcmVcIixcbiAgICAgICAgICBcInNvbGlkLXByZXNlbmNlXCIsXG4gICAgICAgICAgXCJzb2xpZC1wcmV2ZW50LXNjcm9sbFwiLFxuICAgICAgICBdLFxuICAgICAgfSxcbiAgICB9LFxuICB9LFxufSk7XG4iXSwKICAibWFwcGluZ3MiOiAiO0FBQTBQLFNBQVMsb0JBQW9CO0FBQ3ZSLE9BQU8saUJBQWlCO0FBRXhCLElBQU8sc0JBQVEsYUFBYTtBQUFBLEVBQzFCLFNBQVMsQ0FBQyxZQUFZLENBQUM7QUFBQSxFQUN2QixTQUFTO0FBQUEsSUFDUCxZQUFZLENBQUMsZUFBZSxTQUFTO0FBQUE7QUFBQTtBQUFBO0FBQUEsSUFJckMsUUFBUSxDQUFDLFlBQVksZ0JBQWdCLGdCQUFnQjtBQUFBLEVBQ3ZEO0FBQUEsRUFDQSxRQUFRO0FBQUEsSUFDTixNQUFNO0FBQUEsSUFDTixZQUFZO0FBQUEsSUFDWixRQUFRO0FBQUEsTUFDTixhQUFhLENBQUMsb0NBQW9DO0FBQUEsSUFDcEQ7QUFBQSxFQUNGO0FBQUEsRUFDQSxPQUFPO0FBQUEsSUFDTCxRQUFRO0FBQUEsSUFDUixRQUFRO0FBQUEsSUFDUixlQUFlO0FBQUEsTUFDYixRQUFRO0FBQUEsUUFDTixhQUFhLElBQUk7QUFDZixjQUFJLENBQUMsR0FBRyxTQUFTLGNBQWMsRUFBRyxRQUFPO0FBQ3pDLGNBQUksR0FBRyxTQUFTLFVBQVUsRUFBRyxRQUFPO0FBQ3BDLGNBQUksR0FBRyxTQUFTLGFBQWEsRUFBRyxRQUFPO0FBQ3ZDLGNBQUksR0FBRyxTQUFTLFFBQVEsS0FBSyxHQUFHLFNBQVMsV0FBVyxLQUFLLEdBQUcsU0FBUyxjQUFjLEdBQUc7QUFDcEYsbUJBQU87QUFBQSxVQUNUO0FBQ0EsY0FBSSxHQUFHLFNBQVMsVUFBVSxFQUFHLFFBQU87QUFDcEMsaUJBQU87QUFBQSxRQUNUO0FBQUEsTUFDRjtBQUFBLElBQ0Y7QUFBQSxFQUNGO0FBQUEsRUFDQSxNQUFNO0FBQUEsSUFDSixhQUFhO0FBQUEsSUFDYixZQUFZO0FBQUEsSUFDWixTQUFTLENBQUMsd0JBQXdCO0FBQUEsSUFDbEMsS0FBSztBQUFBLE1BQ0gsU0FBUztBQUFBLElBQ1g7QUFBQSxJQUNBLFFBQVE7QUFBQSxNQUNOLE1BQU07QUFBQSxRQUNKLFFBQVE7QUFBQSxVQUNOO0FBQUEsVUFDQTtBQUFBLFVBQ0E7QUFBQSxVQUNBO0FBQUEsVUFDQTtBQUFBLFVBQ0E7QUFBQSxVQUNBO0FBQUEsVUFDQTtBQUFBLFVBQ0E7QUFBQSxVQUNBO0FBQUEsUUFDRjtBQUFBLE1BQ0Y7QUFBQSxJQUNGO0FBQUEsRUFDRjtBQUNGLENBQUM7IiwKICAibmFtZXMiOiBbXQp9Cg==
