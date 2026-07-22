/**
 * Machines Space — fleet, mobile devices, and remote control (tasks 9.1–9.3,
 * Req 8.1/8.4).
 *
 * Regions (design.md §6.5): a toolbar (enroll + reconnect + honest stream
 * state), the fleet matrix (a REAL table of DeviceRow with health / latency /
 * docker / tests, Req 17.2), mobile gateway/pairing/device governance, remote
 * desktop, the focused device TerminalPane (keyboard accessible, Req 17.1),
 * and AlertList. Selecting a fleet device opens the device Inspector in the ONE
 * shared Inspector via the `type: "device"` renderer registered on mount.
 *
 * Mobile pairing, listing, and revocation use desktop-runtime commands only;
 * device tokens remain backend-owned and never enter this UI. Fleet deletion
 * and mobile revocation both require deliberate confirmation (Req 8.4).
 *
 * ── ARCHITECTURE INVARIANT (KRIA is the authoritative orchestrator) ──────────
 * KRIA orchestrates; enrolled targets are execution substrates surfaced here
 * for legibility. Live fleet data is READ from the existing fleet SSE/WS stream
 * (`useDeviceStatus`); enrollment and deletion are DISPATCHED through the
 * runtime's own EXISTING commands (`register_new_target`, `delete_target`) —
 * this Space never touches a substrate directly and creates no substrate
 * self-authority. Destructive delete requires a deliberate confirm (Req 8.4).
 * When the fleet service is absent the stream stays idle and the matrix shows
 * honest empty/offline states (Req 20.4). All device text is UNTRUSTED and
 * rendered as escaped text (Solid).
 *
 * Requirements: 8.1, 8.4
 */
import { createEffect, createMemo, createSignal, onCleanup, onMount, Show } from "solid-js";
import { shellStore, notificationStore, machineStore } from "../../stores";
import { currentRoute } from "../router";
import { Button, Confirm, IconButton, StatusDot } from "../../kit";
import { Icon } from "../../components/Icon";
import { useDeviceStatus, type DeviceTargetView } from "../../hooks/useDeviceStatus";
import {
  AlertList,
  EnrollWizard,
  FleetMatrix,
  MobileDevicesPanel,
  RemoteDesktopCanvas,
  TerminalPane,
  registerDeviceInspector,
  deriveControllerBaseUrl,
  deriveFleetLeaseId,
  mapRegistryTargets,
  type EnrollRequest,
  type EnrollResult,
} from "./machines";
import "./machines/machines.css";

/** Map the live-stream state to a StatusDot tone (dot is supplementary to text). */
function streamTone(state: string): "online" | "busy" | "error" | "offline" {
  if (state === "online") return "online";
  if (state === "connecting") return "busy";
  if (state === "degraded") return "error";
  return "offline";
}

function streamLabel(state: string): string {
  switch (state) {
    case "online":
      return "Live";
    case "connecting":
      return "Connecting";
    case "degraded":
      return "Offline";
    case "stopped":
      return "Paused";
    default:
      return "Local only";
  }
}

function noteId(): string {
  return `machines-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

export default function MachinesSpace() {
  // Register the device Inspector body (type "device") so selecting a DeviceRow
  // opens it in the ONE shared Inspector (Req 1.6). Disposed on unmount.
  onCleanup(registerDeviceInspector());

  // Derive the live-stream inputs from the EXISTING ironclad status snapshot.
  const controllerBaseUrl = createMemo(() =>
    deriveControllerBaseUrl(machineStore.ironcladStatus(), machineStore.settings()),
  );
  const fleetLeaseId = createMemo(() =>
    deriveFleetLeaseId(machineStore.ironcladStatus(), machineStore.settings()),
  );
  const initialTargets = createMemo(() => mapRegistryTargets(machineStore.ironcladStatus()));

  const fleet = useDeviceStatus({
    commanderBaseUrl: controllerBaseUrl,
    initialTargets,
    leaseId: fleetLeaseId,
    heartbeatIntervalMs: 15_000,
    autoStart: false,
  });

  // Load the fleet snapshot + start the stream only when a controller exists
  // (otherwise stay idle — no false "offline" alarms, Req 20.4).
  onMount(() => {
    void machineStore.loadFleetStatus();
    if (controllerBaseUrl()) fleet.start();
  });

  const [wizardOpen, setWizardOpen] = createSignal(false);
  const [pendingDelete, setPendingDelete] = createSignal<DeviceTargetView | null>(null);
  const [deletingIds, setDeletingIds] = createSignal<Set<string>>(new Set());

  const selectedTargetId = createMemo(() => {
    const target = shellStore.inspectorTarget();
    return target?.type === "device" ? target.id : null;
  });

  const focusedDevice = createMemo<DeviceTargetView | null>(() => {
    const id = fleet.focusedTargetId();
    if (!id) return null;
    return fleet.targets().find((t) => t.targetId === id) ?? null;
  });

  // Open the device Inspector on the shared Inspector (Req 1.6). A user row
  // click leaves activeElement on the control (correct owner via the default);
  // the deep-link path passes an explicit region owner (§20.3/§20.4) since it
  // is programmatic.
  function inspect(device: DeviceTargetView, opts?: { region?: boolean }) {
    shellStore.openInspector(
      "device",
      device.targetId,
      {
        device,
        testResult: fleet.lastTestResultByTarget(device.targetId),
      },
      opts?.region ? { regionSelector: '[data-space="machines"]' } : undefined,
    );
  }

  // Device palette/hash deep links wait for the authoritative fleet snapshot,
  // then open the shared Inspector and focus the matching matrix row.
  let handledDeviceRoute: string | null = null;
  createEffect(() => {
    const route = currentRoute();
    const targets = fleet.targets();
    if (route.space !== "machines" || route.segment !== "device" || !route.entityId) return;
    const device = targets.find((target) => target.targetId === route.entityId);
    if (!device) return;
    const routeKey = `${route.space}/${route.segment}/${route.entityId}`;
    if (handledDeviceRoute === routeKey) return;

    inspect(device, { region: true });
    queueMicrotask(() => {
      if (currentRoute().entityId !== device.targetId) return;
      const row = Array.from(
        document.querySelectorAll<HTMLElement>("[data-target-id]"),
      ).find((element) => element.dataset.targetId === device.targetId);
      row?.scrollIntoView?.({ block: "center" });
      row?.querySelector<HTMLElement>("button")?.focus({ preventScroll: true });
      handledDeviceRoute = routeKey;
    });
  });

  function toggleTerminal(targetId: string) {
    fleet.focusTarget(fleet.focusedTargetId() === targetId ? null : targetId);
  }

  // Dispatch enrollment through the runtime's EXISTING `register_new_target`.
  async function onEnroll(request: EnrollRequest): Promise<EnrollResult> {
    const result = await machineStore.enrollTarget(request);
    if (result.ok) {
      fleet.reconnectNow();
      notificationStore.push({
        id: noteId(),
        level: "success",
        message: `Device “${request.displayName}” enrolled.`,
        source: "machines",
      });
      return { ok: true };
    }
    return {
      ok: false,
      title: result.code === "unavailable" ? "Fleet service unavailable" : "Enrollment failed",
      message: result.message,
    };
  }

  // Destructive delete — dispatched only AFTER a deliberate confirm (Req 8.4).
  async function confirmDelete() {
    const device = pendingDelete();
    if (!device) return;
    setPendingDelete(null);
    setDeletingIds((prev) => new Set(prev).add(device.targetId));
    const result = await machineStore.deleteTargetById(device.targetId);
    if (result.ok) {
      fleet.removeTarget(device.targetId);
      if (selectedTargetId() === device.targetId) shellStore.closeInspector();
      notificationStore.push({
        id: noteId(),
        level: "info",
        message: `Device “${device.displayName}” removed from the fleet.`,
        source: "machines",
      });
    } else {
      notificationStore.push({
        id: noteId(),
        level: "error",
        message: `Could not delete “${device.displayName}”.`,
        detail: result.message,
        source: "machines",
      });
    }
    setDeletingIds((prev) => {
      const next = new Set(prev);
      next.delete(device.targetId);
      return next;
    });
  }

  // Docker evals dispatch through the fleet commander (needs an active lease).
  async function runDocker(targetId: string) {
    const commander = controllerBaseUrl();
    const leaseId = fleetLeaseId();
    if (!commander || !leaseId) return;
    try {
      const response = await fetch(`${commander}/api/fleet/docker-evals`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ lease_id: leaseId, target_id: targetId }),
      });
      if (!response.ok) throw new Error(`status ${response.status}`);
      notificationStore.push({
        id: noteId(),
        level: "info",
        message: `Docker evals triggered for ${targetId}.`,
        source: "machines",
      });
    } catch (error) {
      notificationStore.push({
        id: noteId(),
        level: "error",
        message: `Docker eval failed for ${targetId}.`,
        detail: error instanceof Error ? error.message : String(error),
        source: "machines",
      });
    }
  }

  const dockerDisabled = createMemo(() => !controllerBaseUrl() || !fleetLeaseId());

  return (
    <section class="kria-machines" data-space="machines" aria-label="Machines">
      <header class="kria-machines__header">
        <h1 class="kria-machines__title">Machines</h1>
        <p class="kria-machines__subtitle">
          Fleet, VM, and remote control — every machine KRIA touches, in one place.
        </p>
      </header>

      <div class="kria-machines__toolbar">
        <Button variant="primary" size="sm" onClick={() => setWizardOpen(true)}>
          <Icon name="plus" size={14} aria-hidden /> Enroll device
        </Button>
        <IconButton
          icon="refresh-cw"
          label="Reconnect fleet streams"
          size="sm"
          onClick={() => fleet.reconnectNow()}
        />
        <span class="kria-machines__toolbar-spacer" />
        <span class="kria-machines__stream">
          <StatusDot tone={streamTone(fleet.streamState())} label={streamLabel(fleet.streamState())} />
          <span>{streamLabel(fleet.streamState())}</span>
        </span>
        <Show when={fleet.lastError()}>
          <span class="kria-machines__stream" role="status">
            <Icon name="info" size={13} aria-hidden /> {fleet.lastError()}
          </span>
        </Show>
      </div>

      <div class="kria-machines__region" data-mode-region="fleet" aria-label="Fleet matrix">
        <FleetMatrix
          fleet={fleet.targets()}
          streamState={fleet.streamState()}
          focusedTerminalTargetId={fleet.focusedTargetId()}
          selectedTargetId={selectedTargetId()}
          testResultFor={(id) => fleet.lastTestResultByTarget(id)}
          onInspect={inspect}
          onToggleTerminal={toggleTerminal}
          onRequestDelete={setPendingDelete}
          onRunDocker={runDocker}
          dockerDisabled={dockerDisabled()}
          dockerDisabledReason="Docker evals need an active fleet lease"
          deletingTargetIds={deletingIds()}
        />
      </div>

      <div class="kria-machines__region" data-mode-region="mobile" aria-label="Mobile pairing and devices">
        <MobileDevicesPanel />
      </div>

      <div class="kria-machines__region" data-mode-region="remote" aria-label="Remote desktop">
        <RemoteDesktopCanvas />
      </div>

      <div class="kria-machines__region" data-mode-region="terminal" aria-label="Terminal">
        <h2 class="kria-machines__region-title">Terminal</h2>
        <TerminalPane
          device={focusedDevice()}
          lines={fleet.focusedTerminalLines()}
          onDetach={() => fleet.focusTarget(null)}
        />
      </div>

      <div class="kria-machines__region" data-mode-region="alerts" aria-label="Alerts">
        <h2 class="kria-machines__region-title">Alerts</h2>
        <AlertList alerts={fleet.alerts()} />
      </div>

      <EnrollWizard open={wizardOpen()} onOpenChange={setWizardOpen} onEnroll={onEnroll} />

      {/* Deliberate confirm for the destructive delete (Req 8.4). */}
      <Confirm
        open={pendingDelete() !== null}
        onOpenChange={(open) => {
          if (!open) setPendingDelete(null);
        }}
        title="Delete device?"
        message={`This removes “${pendingDelete()?.displayName ?? "this device"}” from the fleet. KRIA will no longer orchestrate it until it is enrolled again.`}
        confirmLabel="Delete device"
        risk="danger"
        onConfirm={confirmDelete}
      />
    </section>
  );
}
