import { Component, createSignal, For, Show } from "solid-js";
import { SCANCODE, type RdpInputHandle } from "../rdpInput";

/**
 * On-screen modifier + special-key bar for the remote desktop.
 *
 * Row 1: sticky modifiers (Ctrl/Alt/Shift/Super), navigation/control keys, and
 * arrows. Row 2 (toggle): function keys F1–F12. Sticky modifiers apply to the
 * next key tap and then auto-release, with their on/off state indicated.
 *
 * All keys are ≥44px touch targets and carry `aria-label`s.
 */
export interface RdKeyboardBarProps {
  input: () => RdpInputHandle | null;
}

const F_KEYS = [
  SCANCODE.F1,
  SCANCODE.F2,
  SCANCODE.F3,
  SCANCODE.F4,
  SCANCODE.F5,
  SCANCODE.F6,
  SCANCODE.F7,
  SCANCODE.F8,
  SCANCODE.F9,
  SCANCODE.F10,
  SCANCODE.F11,
  SCANCODE.F12,
];

const RdKeyboardBar: Component<RdKeyboardBarProps> = (props) => {
  const [ctrlOn, setCtrlOn] = createSignal(false);
  const [altOn, setAltOn] = createSignal(false);
  const [shiftOn, setShiftOn] = createSignal(false);
  const [fnOpen, setFnOpen] = createSignal(false);

  const input = () => props.input();

  const toggleMod = (sc: number, get: () => boolean, set: (v: boolean) => void) => {
    const next = !get();
    set(next);
    input()?.setKey(sc, next);
  };

  // Tap a key, then auto-release any held sticky modifiers.
  const tap = (sc: number) => {
    input()?.tapKey(sc);
    if (ctrlOn()) {
      input()?.setKey(SCANCODE.ControlLeft, false);
      setCtrlOn(false);
    }
    if (altOn()) {
      input()?.setKey(SCANCODE.AltLeft, false);
      setAltOn(false);
    }
    if (shiftOn()) {
      input()?.setKey(SCANCODE.ShiftLeft, false);
      setShiftOn(false);
    }
  };

  return (
    <div class="mobile-desktop-keybar">
      <div class="mobile-desktop-keybar-row">
        <button
          classList={{ active: ctrlOn() }}
          aria-label="Control"
          aria-pressed={ctrlOn()}
          onClick={() => toggleMod(SCANCODE.ControlLeft, ctrlOn, setCtrlOn)}
        >
          Ctrl
        </button>
        <button
          classList={{ active: altOn() }}
          aria-label="Alt"
          aria-pressed={altOn()}
          onClick={() => toggleMod(SCANCODE.AltLeft, altOn, setAltOn)}
        >
          Alt
        </button>
        <button
          classList={{ active: shiftOn() }}
          aria-label="Shift"
          aria-pressed={shiftOn()}
          onClick={() => toggleMod(SCANCODE.ShiftLeft, shiftOn, setShiftOn)}
        >
          Shift
        </button>
        <button aria-label="Super / Windows key" onClick={() => tap(SCANCODE.MetaLeft)}>
          Win
        </button>
        <button aria-label="Tab" onClick={() => tap(SCANCODE.Tab)}>
          Tab
        </button>
        <button aria-label="Escape" onClick={() => tap(SCANCODE.Escape)}>
          Esc
        </button>
        <button aria-label="Arrow up" onClick={() => tap(SCANCODE.ArrowUp)}>
          ↑
        </button>
        <button aria-label="Arrow down" onClick={() => tap(SCANCODE.ArrowDown)}>
          ↓
        </button>
        <button aria-label="Arrow left" onClick={() => tap(SCANCODE.ArrowLeft)}>
          ←
        </button>
        <button aria-label="Arrow right" onClick={() => tap(SCANCODE.ArrowRight)}>
          →
        </button>
        <button
          classList={{ active: fnOpen() }}
          aria-label="Toggle function keys"
          aria-pressed={fnOpen()}
          onClick={() => setFnOpen(!fnOpen())}
        >
          Fn
        </button>
      </div>
      <Show when={fnOpen()}>
        <div class="mobile-desktop-keybar-row">
          <For each={F_KEYS}>
            {(sc, i) => (
              <button aria-label={`F${i() + 1}`} onClick={() => tap(sc)}>
                F{i() + 1}
              </button>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
};

export default RdKeyboardBar;
