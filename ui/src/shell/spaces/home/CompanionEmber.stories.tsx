import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { createSignal } from "solid-js";

import CompanionEmber from "./CompanionEmber";
import { coreStore } from "../../../stores";
import type { CoreState } from "../../../stores/coreStore";

/**
 * CompanionEmber — the floating cross-application Companion ember (design §8/§9,
 * Requirements 13.4, 15.1–15.5). These stories drive the REAL component (task
 * 8.3) with the `active`/`enabled`/`onReturn` overrides so the workbench stays
 * deterministic without steering the whole View-Mode machine or the native
 * window. The ember mirrors `coreStore.state()` read-only (Req 15.1), brightens
 * ONLY for meaningful needs (Req 15.2), and its "Return to KRIA" is a stubbed
 * no-op here (real return funnels through `requestWindowMode`, Req 15.3).
 *
 * Requirements: 13.4, 15.1, 15.2, 15.3, 15.4, 15.5 (and 16.4 — workbench parity).
 */
const meta = {
  title: "Spaces/Home/CompanionEmber",
  component: CompanionEmber,
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-shell" data-window-mode="companion" style={{ height: "480px", position: "relative" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof CompanionEmber>;

export default meta;
type Story = StoryObj<typeof meta>;

const onReturn = () => console.info("[CompanionEmber story] return → requestWindowMode(prior)");

function withCoreState(state: CoreState) {
  coreStore.reset();
  coreStore.setState(state);
}

/** Resting ember — mirrors an idle Core, dim, no brighten (Req 15.1/15.2). */
export const Resting: Story = {
  render: () => {
    withCoreState("idle");
    return <CompanionEmber active={() => true} enabled={() => true} onReturn={onReturn} />;
  },
};

/** Meaningful need — a blocked Core brightens the ember (Req 15.2). */
export const NeedsYou: Story = {
  render: () => {
    withCoreState("blocked");
    return <CompanionEmber active={() => true} enabled={() => true} onReturn={onReturn} />;
  },
};

/** Working — an ordinary active state must NOT brighten (idle chatter, Req 15.2). */
export const WorkingCalm: Story = {
  render: () => {
    withCoreState("thinking");
    return <CompanionEmber active={() => true} enabled={() => true} onReturn={onReturn} />;
  },
};

/** Opted out — the one-setting opt-out renders nothing (Req 15.4). */
export const OptedOut: Story = {
  render: () => {
    withCoreState("idle");
    return <CompanionEmber active={() => true} enabled={() => false} onReturn={onReturn} />;
  },
};

/** Live — toggle the mirrored Core state to watch the ember brighten/settle. */
export const Live: Story = {
  render: () => {
    const [needy, setNeedy] = createSignal(false);
    const sync = () => withCoreState(needy() ? "blocked" : "idle");
    sync();
    return (
      <>
        <CompanionEmber active={() => true} enabled={() => true} onReturn={onReturn} />
        <button
          type="button"
          style={{ position: "absolute", top: "16px", left: "16px" }}
          onClick={() => {
            setNeedy((n) => !n);
            sync();
          }}
        >
          Toggle meaningful need
        </button>
      </>
    );
  },
};
