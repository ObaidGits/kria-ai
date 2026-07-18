/**
 * layoutSettle tests (task 6.4) — the settle/stop decision for the layout
 * worker. §5.4 hard rule: the layout runs in a Worker and STOPS when settled →
 * the scene becomes static (no perpetual simulation). ngraph itself is
 * worker/browser-only; this verifies the stop decision in isolation.
 */
import { describe, it, expect } from "vitest";
import { SettleTracker } from "./layoutSettle";

describe("SettleTracker", () => {
  it("settles after enough consecutive quiet steps (converged)", () => {
    const t = new SettleTracker({ epsilon: 0.01, quietStepsRequired: 3, maxSteps: 100 });
    expect(t.step(1)).toBe(false); // moving
    expect(t.step(0.005)).toBe(false); // quiet 1
    expect(t.step(0.005)).toBe(false); // quiet 2
    expect(t.step(0.005)).toBe(true); // quiet 3 → settled
    expect(t.settled).toBe(true);
    expect(t.reason).toBe("quiet");
    expect(t.stepCount).toBe(4);
  });

  it("resets the quiet run when movement spikes again", () => {
    const t = new SettleTracker({ epsilon: 0.01, quietStepsRequired: 2, maxSteps: 100 });
    t.step(0.001); // quiet 1
    t.step(5); // spike → reset
    expect(t.settled).toBe(false);
    t.step(0.001); // quiet 1 again
    expect(t.settled).toBe(false);
    t.step(0.001); // quiet 2 → settled
    expect(t.settled).toBe(true);
  });

  it("stops at the max-step ceiling even if never quiet", () => {
    const t = new SettleTracker({ epsilon: 0.01, quietStepsRequired: 999, maxSteps: 5 });
    let settled = false;
    for (let i = 0; i < 10 && !settled; i++) settled = t.step(100);
    expect(t.settled).toBe(true);
    expect(t.reason).toBe("max-steps");
    expect(t.stepCount).toBe(5);
  });

  it("is idempotent once settled (always reports true, no extra steps)", () => {
    const t = new SettleTracker({ epsilon: 0.01, quietStepsRequired: 1, maxSteps: 100 });
    t.step(0.001); // settled
    const countAfterSettle = t.stepCount;
    expect(t.step(100)).toBe(true);
    expect(t.stepCount).toBe(countAfterSettle); // no further stepping
  });

  it("can be reset for a fresh run", () => {
    const t = new SettleTracker({ epsilon: 0.01, quietStepsRequired: 1, maxSteps: 100 });
    t.step(0.001);
    expect(t.settled).toBe(true);
    t.reset();
    expect(t.settled).toBe(false);
    expect(t.stepCount).toBe(0);
    expect(t.reason).toBeNull();
  });
});
