import { describe, it, expect } from "vitest";
import {
  deriveControllerBaseUrl,
  deriveFleetLeaseId,
  mapRegistryTargets,
} from "./fleetWiring";

describe("fleetWiring — deriveControllerBaseUrl (Req 8.1/20.4)", () => {
  it("uses an explicit controller base url and strips trailing slash + /v1", () => {
    const status = { fleet: { pool_packet: { controller_base_url: "https://fleet.local:8443/v1/" } } };
    expect(deriveControllerBaseUrl(status, null)).toBe("https://fleet.local:8443");
  });

  it("falls back to the local server host/port", () => {
    const settings = { server: { host: "192.168.1.10", port: 8899 } };
    expect(deriveControllerBaseUrl(null, settings)).toBe("http://192.168.1.10:8899");
  });

  it("normalizes 0.0.0.0 to 127.0.0.1", () => {
    const settings = { server: { host: "0.0.0.0", port: "7070" } };
    expect(deriveControllerBaseUrl(null, settings)).toBe("http://127.0.0.1:7070");
  });

  it("returns null when nothing is configured (→ idle, no false alarms)", () => {
    expect(deriveControllerBaseUrl(null, null)).toBeNull();
    expect(deriveControllerBaseUrl({}, {})).toBeNull();
  });
});

describe("fleetWiring — deriveFleetLeaseId (Req 8.1/20.4)", () => {
  it("reads the active lease id from the pool packet", () => {
    const status = { fleet: { pool_packet: { active_lease_id: "lease-123" } } };
    expect(deriveFleetLeaseId(status, null)).toBe("lease-123");
  });

  it("falls back to a settings-configured lease id", () => {
    const settings = { fleet: { lease_id: "lease-cfg" } };
    expect(deriveFleetLeaseId(null, settings)).toBe("lease-cfg");
  });

  it("returns null when no lease is active", () => {
    expect(deriveFleetLeaseId({ fleet: {} }, null)).toBeNull();
  });
});

describe("fleetWiring — mapRegistryTargets (Req 8.1)", () => {
  it("maps enrolled targets with sensible defaults", () => {
    const status = {
      fleet: {
        enrolled_targets: [
          { target_id: "t1", display_name: "Office VM", mode: "ssh_bootstrap" },
        ],
      },
    };
    const rows = mapRegistryTargets(status);
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({
      targetId: "t1",
      displayName: "Office VM",
      state: "unknown",
      dockerHealth: "unknown",
      healthScore: 1,
    });
  });

  it("prefers live connection-control state over plain enrolled rows", () => {
    const status = {
      fleet: {
        enrolled_targets: [{ target_id: "t1", display_name: "VM" }],
        connection_control_targets: [
          {
            target_id: "t2",
            display_name: "Live VM",
            state: "ready",
            health_score: 0.9,
            latency_ewma_ms: 42,
            docker_health: "pass",
          },
        ],
      },
    };
    const rows = mapRegistryTargets(status);
    const live = rows.find((r) => r.targetId === "t2")!;
    expect(live.state).toBe("ready");
    expect(live.dockerHealth).toBe("pass");
    expect(live.latencyEwmaMs).toBe(42);
  });

  it("deduplicates by target id (first wins)", () => {
    const status = {
      fleet: {
        enrolled_targets: [
          { target_id: "dup", display_name: "First" },
          { target_id: "dup", display_name: "Second" },
        ],
      },
    };
    const rows = mapRegistryTargets(status);
    expect(rows).toHaveLength(1);
    expect(rows[0].displayName).toBe("First");
  });

  it("returns [] when there is no fleet snapshot (graceful, Req 20.4)", () => {
    expect(mapRegistryTargets(null)).toEqual([]);
    expect(mapRegistryTargets({})).toEqual([]);
  });
});
