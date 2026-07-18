/** Dev-only deterministic browser harness for final flow-map E2E. */
import {
  approvalStore,
  eventBus,
  memoryStore,
  voiceStore,
} from "../stores";
import { setWindowPresentationActive } from "../windowing/detachableSurfaces";

const CORRECTION_FACT = {
  id: "e2e-memory-correction",
  content: "Project Atlas launches on Monday",
  confidence: 0.72,
  worth: 0.6,
  staleness: 0.1,
  source: "conversation",
  createdAt: 1,
  updatedAt: 1,
  tags: ["project"],
};

const VOICE_FACT = {
  id: "e2e-voice-memory",
  content: "Voice-approved deployment completed with verification",
  confidence: 0.95,
  worth: 0.9,
  staleness: 0,
  source: "verified-action",
  createdAt: 2,
  updatedAt: 2,
  tags: ["voice", "verified"],
};

const MULTI_WINDOW_APPROVAL = {
  id: "e2e-multi-window-approval",
  source: "tool-hitl" as const,
  title: "Approve remote maintenance",
  description: "KRIA needs permission before the verified maintenance step.",
  risk: "yellow" as const,
  effects: ["Run one bounded maintenance action"],
  routing: { requestId: "e2e-maintenance-request" },
};

type BackendCall = { command: string; args?: Record<string, unknown> };
type FixtureBackend = {
  calls: BackendCall[];
  setMemoryEntries(entries: Array<Record<string, unknown>>): void;
};

function backend(): FixtureBackend | undefined {
  return (window as unknown as { __KRIA_E2E_BACKEND__?: FixtureBackend }).__KRIA_E2E_BACKEND__;
}

function syncFixtureMemory(facts: typeof CORRECTION_FACT[]): void {
  backend()?.setMemoryEntries(facts);
}

function addMultiWindowApproval(): void {
  approvalStore.addFromEnvelope(MULTI_WINDOW_APPROVAL);
}

export function installE2EHarness(): void {
  const target = window as unknown as { __KRIA_E2E__?: Record<string, unknown> };
  if (target.__KRIA_E2E__) return;

  window.addEventListener("storage", (event) => {
    if (event.key === "kria-e2e-pending-approval" && event.newValue) {
      addMultiWindowApproval();
    }
    if (event.key === "kria-e2e-approval-resolution" && event.newValue) {
      const value = JSON.parse(event.newValue) as { id?: string };
      if (value.id) approvalStore.dismiss(value.id);
    }
  });

  target.__KRIA_E2E__ = {
    seedVoiceApproval() {
      voiceStore.activate();
      voiceStore.setState("listening");
      voiceStore.setTranscript("Deploy the verified preview and remember the result", false);
      approvalStore.addFromEnvelope({
        id: "e2e-voice-approval",
        source: "tool-hitl",
        title: "Deploy verified preview",
        description: "Voice intent resolved to a bounded deployment requiring approval.",
        risk: "yellow",
        effects: ["Deploy preview", "Verify health", "Record verified outcome"],
        evidence: "Voice transcript matched the preview deployment intent.",
        routing: { requestId: "e2e-voice-request" },
      });
    },
    completeVoiceExecution() {
      voiceStore.setState("thinking");
      const facts = [...memoryStore.facts().filter((f) => f.id !== VOICE_FACT.id), VOICE_FACT];
      syncFixtureMemory(facts);
      memoryStore.setFacts(facts);
      eventBus.emit("memory:updated", { factId: VOICE_FACT.id });
      voiceStore.setTranscript("Deployment completed, verified, and remembered", false);
      voiceStore.setState("speaking");
    },
    seedMemoryCorrection() {
      syncFixtureMemory([CORRECTION_FACT]);
      memoryStore.setFacts([CORRECTION_FACT]);
    },
    seedMultiWindowApproval() {
      addMultiWindowApproval();
      localStorage.setItem("kria-e2e-pending-approval", String(Date.now()));
    },
    setWindowActive(value: boolean) {
      setWindowPresentationActive(value);
    },
    stressTelemetry(count = 2_000) {
      for (let index = 0; index < count; index += 1) {
        eventBus.emit("observatory:telemetry", { metric: "cpu", value: index % 100, ts: index });
      }
    },
    backendCalls() {
      return backend()?.calls ?? [];
    },
    pendingApprovalCount() {
      return approvalStore.pendingCount();
    },
  };
}
