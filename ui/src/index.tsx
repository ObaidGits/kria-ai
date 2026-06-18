/* @refresh reload */
import { render } from "solid-js/web";
import { ErrorBoundary } from "solid-js";
// Global styles are imported here (the idiomatic Vite way) rather than via a
// <link rel="stylesheet" href="/src/styles/global.css"> in index.html. In Vite
// dev a bare .css URL is served as a JS module (text/javascript), so loading it
// through a stylesheet <link> caused a MIME mismatch
// ("'text/css' is not a valid JavaScript MIME type") and a blank, unstyled page.
// Importing it from the JS entry makes Vite handle it correctly in dev (HMR
// style injection) and prod (extracted CSS chunk).
import "./styles/global.css";
import "./styles/mobile.css";
import { lazy } from "solid-js";
import BootError from "./components/BootError";
import { endUiMeasure, startUiMeasure } from "./utils/performance";

const root = document.getElementById("root");
if (!root) throw new Error("Root element not found");

// Route selection: the standalone mobile PWA lives at `/m` (the manifest's
// start_url), independent of the Tauri desktop app so it runs in a plain phone
// browser. We lazy-load each surface so the mobile path NEVER pulls the desktop
// app's stores (which call Tauri `invoke` and throw in a plain browser).
const isMobileShell =
  typeof window !== "undefined" && window.location.pathname.replace(/\/+$/, "").endsWith("/m");

const RootComponent = isMobileShell
  ? lazy(() => import("./mobile/MobileApp"))
  : lazy(() => import("./App"));

// Top-level error boundary: any render-time throw is caught here and shown as a
// visible, recoverable error instead of a blank window.
const renderStart = startUiMeasure("app-render");
render(
  () => (
    <ErrorBoundary fallback={(err, reset) => <BootError err={err} reset={reset} />}>
      <RootComponent />
    </ErrorBoundary>
  ),
  root,
);
endUiMeasure("app-render", renderStart);
