/**
 * Summon public surface (Req 2.5, 18.2).
 *
 * `initSummon()` is wired once in AppShell.onMount; `summon()` is the shared
 * action the tray/global-hotkey/Mini reuse. See ./summon.ts for the
 * enhancement-with-fallback design.
 */
export {
  summon,
  initSummon,
  disposeSummon,
  isSummonHotkey,
  isTypingTarget,
  SUMMON_COMMAND,
  SUMMON_EVENT,
} from "./summon";
