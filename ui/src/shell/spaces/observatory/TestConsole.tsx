import { createSignal } from "solid-js";
import { Button, Select } from "../../../kit";
import type { DataAuthority, TestRunState } from "../../../stores";
import { HonestyBadge } from "./HonestyBadge";

export function TestConsole(props: {
  authority: DataAuthority;
  state: TestRunState;
  onStart: (mode: "SAFE" | "SMOKE") => Promise<boolean>;
  onStop: () => Promise<boolean>;
  onRefresh: () => Promise<void>;
}) {
  const [mode, setMode] = createSignal<"SAFE" | "SMOKE">("SAFE");
  const [busy, setBusy] = createSignal(false);
  const [message, setMessage] = createSignal<string | null>(null);

  async function run() {
    setBusy(true);
    const ok = await props.onStart(mode());
    setMessage(ok ? "Test run started through KRIA runtime." : "Test runner unavailable; nothing started.");
    setBusy(false);
  }
  async function stop() {
    setBusy(true);
    const ok = await props.onStop();
    setMessage(ok ? "Stop sent to active test run." : "No test run was stopped.");
    setBusy(false);
  }

  return (
    <section class="kria-observatory__test-console" aria-labelledby="test-console-heading">
      <div class="kria-observatory__region-head">
        <h2 id="test-console-heading">Test console</h2>
        <HonestyBadge authority={props.authority} />
      </div>
      <p>Bounded diagnostic profiles only. Destructive mode stays unavailable here.</p>
      <div class="kria-observatory__test-controls">
        <Select label="Test profile" value={mode()} options={[
          { value: "SAFE", label: "Safe" }, { value: "SMOKE", label: "Smoke" },
        ]} onChange={(value) => value && setMode(value as "SAFE" | "SMOKE")} disabled={props.state.running || busy()} />
        <Button onClick={run} disabled={props.state.running || busy()}>Run tests</Button>
        <Button variant="danger" onClick={stop} disabled={!props.state.running || busy()}>Stop run</Button>
        <Button variant="secondary" onClick={() => void props.onRefresh()} disabled={busy()}>Refresh</Button>
      </div>
      <dl class="kria-observatory__test-state">
        <div><dt>Status</dt><dd>{props.state.running ? "Running" : "Idle"}</dd></div>
        <div><dt>Profile</dt><dd>{props.state.mode ?? "None"}</dd></div>
        <div><dt>Run</dt><dd>{props.state.run_label ?? "None"}</dd></div>
      </dl>
      {message() && <p role="status" aria-live="polite">{message()}</p>}
    </section>
  );
}
