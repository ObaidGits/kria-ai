/**
 * Voice surface barrel (task 5.1, Req 12.1). One stable import surface for the
 * compact VoiceSurface overlay.
 */
export { VoiceSurface, voicePhaseToCoreState } from "./VoiceSurface";
export type { VoiceSurfaceProps } from "./VoiceSurface";
export { VoiceModeSwitcher } from "./VoiceModeSwitcher";
export { VoiceSetupGuide, openVoiceSetupGuide } from "./VoiceSetupGuide";
export { WakeWordTest, WakeWordTestView, wakeTestStatusMeta } from "./WakeWordTest";
export type {
  WakeWordTestProps,
  WakeWordTestViewProps,
  WakeTestStatus,
  WakeTestStatusMeta,
} from "./WakeWordTest";
