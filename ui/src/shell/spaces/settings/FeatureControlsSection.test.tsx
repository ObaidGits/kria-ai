import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";

const bridgeInvoke = vi.hoisted(() => vi.fn());
vi.mock("../../../bridge/invoke", () => ({ bridgeInvoke, bridgeInvokeOptional: vi.fn(async () => null) }));

import { featureControlsStore } from "../../../stores/featureControlsStore";
import { setLocale } from "../../../stores/i18n";
import { FeatureControlsSection } from "./FeatureControlsSection";

const validControl = {
  id: "indexing",
  label: "Indexing",
  description: "Keeps local search data ready.",
  desiredEnabled: true,
  state: "starting",
  detail: "Loading local index",
} as const;

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

beforeEach(() => {
  featureControlsStore.dispose();
  featureControlsStore.setControls([]);
  setLocale("en");
  bridgeInvoke.mockReset();
  bridgeInvoke.mockResolvedValue({ ok: true, data: [validControl] });
});

afterEach(() => {
  cleanup();
  featureControlsStore.dispose();
  setLocale("en");
  vi.restoreAllMocks();
});

describe("FeatureControlsSection", () => {
  it("shows desired state, runtime state, detail, and locks transitional switches", async () => {
    render(() => <FeatureControlsSection />);

    expect(screen.getByRole("heading", { name: "Features & Services" })).toBeInTheDocument();
    const control = await screen.findByRole("switch", { name: "Indexing: On" });
    expect(control).toBeChecked();
    expect(control).toBeDisabled();
    expect(screen.getByText("Starting")).toBeInTheDocument();
    expect(screen.getByText("Loading local index")).toBeInTheDocument();
    expect(bridgeInvoke).toHaveBeenCalledWith("list_feature_controls");
  });

  it.each([
    {
      name: "empty",
      payload: [],
      state: "No feature controls available",
    },
    {
      name: "unavailable",
      payload: null,
      state: "Feature controls unavailable",
    },
    {
      name: "partial",
      payload: [validControl, { ...validControl, id: 42 }],
      state: "Some feature controls unavailable",
    },
  ])("renders a local $name state while unaffected Settings content stays mounted", async ({ payload, state }) => {
    vi.spyOn(console, "warn").mockImplementation(() => undefined);
    bridgeInvoke.mockResolvedValueOnce({ ok: true, data: payload });

    render(() => (
      <main>
        <nav aria-label="Settings groups">General</nav>
        <FeatureControlsSection />
        <section aria-label="Settings preferences">Preferences</section>
      </main>
    ));

    expect(await screen.findByText(state)).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Settings groups" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Settings preferences" })).toBeInTheDocument();
    if (state === "Feature controls unavailable") {
      expect(screen.getByRole("button", { name: "Retry feature controls" })).toBeEnabled();
    }
    if (state === "Some feature controls unavailable") {
      expect(screen.getByText("Valid controls shown: 1. Malformed entries omitted: 1.")).toBeInTheDocument();
      expect(screen.getByRole("switch", { name: "Indexing: On" })).toBeInTheDocument();
    }
  });

  it("names loading, retrying, and recovered states without unmounting sibling content", async () => {
    const initial = deferred<{ ok: false; message: string }>();
    const retry = deferred<{ ok: true; data: readonly [typeof validControl] }>();
    bridgeInvoke
      .mockImplementationOnce(() => initial.promise)
      .mockImplementationOnce(() => retry.promise);

    render(() => (
      <main>
        <FeatureControlsSection />
        <section aria-label="Settings preferences">Preferences</section>
      </main>
    ));

    expect(await screen.findByText("Loading feature controls…")).toBeInTheDocument();
    initial.resolve({ ok: false, message: "Local runtime did not respond." });
    expect(await screen.findByText("Feature controls unavailable")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Retry feature controls" }));
    expect(await screen.findByText("Retrying feature controls…")).toBeInTheDocument();
    retry.resolve({ ok: true, data: [validControl] });

    expect(await screen.findByText("Feature controls recovered")).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Indexing: On" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Settings preferences" })).toBeInTheDocument();
  });

  it("returns focus to the stable section action after retry recovery", async () => {
    const retry = deferred<{ ok: true; data: readonly [typeof validControl] }>();
    bridgeInvoke
      .mockResolvedValueOnce({ ok: false, message: "Local runtime did not respond." })
      .mockImplementationOnce(() => retry.promise);

    render(() => <FeatureControlsSection />);

    const retryAction = await screen.findByRole("button", { name: "Retry feature controls" });
    retryAction.focus();
    expect(document.activeElement).toBe(retryAction);

    fireEvent.click(retryAction);
    expect(await screen.findByText("Retrying feature controls…")).toBeInTheDocument();
    retry.resolve({ ok: true, data: [validControl] });

    expect(await screen.findByText("Feature controls recovered")).toBeInTheDocument();
    const stableAction = screen.getByRole("button", { name: "Refresh" });
    await vi.waitFor(() => expect(document.activeElement).toBe(stableAction));
  });

  it.each([
    ["es", "Controles de funciones no disponibles", "Reintentar controles de funciones"],
    ["de", "Funktionssteuerung nicht verfügbar", "Funktionssteuerung erneut versuchen"],
    ["fr", "Contrôles de fonctionnalités indisponibles", "Réessayer les contrôles de fonctionnalités"],
    ["zh", "功能控制不可用", "重试功能控制"],
    ["ar", "عناصر التحكم بالميزات غير متاحة", "إعادة محاولة عناصر التحكم بالميزات"],
    ["hi", "सुविधा नियंत्रण उपलब्ध नहीं हैं", "सुविधा नियंत्रण फिर आज़माएँ"],
  ])("localizes unavailable recovery controls for %s", async (locale, title, retryLabel) => {
    setLocale(locale);
    bridgeInvoke.mockResolvedValueOnce({ ok: true, data: null });

    render(() => <FeatureControlsSection />);

    expect(await screen.findByText(title)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: retryLabel })).toBeEnabled();
  });
});
