# Voice Runtime Diagnostics — Enhanced Logging Deployed

## Changes Made

- ✅ Added extensive frame-by-frame logging to the v2 capture task
- ✅ Added explicit STT join() diagnostics  
- ✅ All exit paths now logged (cancelled token, STT stream closed, timeout, max utterance, silence detected)
- ✅ Frame count and speech_started state tracked throughout

## Expected Success Log Sequence

When everything works correctly, you should see these logs in order:

1. `voice v2: starting turn loop turn_index=1`
2. `v2 capture task started; awaiting audio frames`
3. `v2 capture awaiting speech start` (repeated every 2 seconds until speech is detected)
4. `v2 capture detected speech start` (when RMS exceeds 0.002 threshold)
5. `v2 pipeline: waiting for STT final transcript; dropped stt_pcm_tx`
6. `v2 pipeline: calling stt_handle.join() to await final transcript`
7. `v2 pipeline: STT handle available, joining...`
8. `v2 pipeline: STT join completed`
9. `voice v2: STT final transcript ready text_len=X engine=...`
10. (Then LLM route, streaming, TTS, playback)

## Expected Logs If Capture Closes Early

- `v2 capture detected end-of-speech silence; finalizing STT stream` — normal case
- `v2 capture: STT stream closed; breaking` — STT channel closed unexpectedly
- `v2 capture timed out waiting for speech` — no voice detected in 12 seconds
- `v2 capture reached max utterance window` — 18 second limit reached

## How to Run Diagnostic Tests

1. **Kill any running kria-desktop process:**
   ```bash
   ps aux | grep kria-desktop
   kill <PID>  # Replace <PID> with the actual process ID
   ```

2. **Run with debug logging enabled:**
   ```bash
   RUST_LOG=info /media/obaid/SSD/KRIA/target/release/kria-desktop
   ```

3. **Wait for initialization:**
   - Watch for `whisper warmup complete` message
   - Watch for `voice v2: starting turn loop`

4. **Test voice interaction:**
   - Click the microphone button in the UI
   - Speak clearly: "Hello, how are you?"
   - Wait for response or error

5. **Collect the logs:**
   - Paste the FULL output starting from when you clicked the mic

## What I'm Looking For

When you provide logs, I need to see:
- **✓ Does "v2 capture task started" appear?** → Capture task is running
- **✓ Does "v2 capture detected speech start" appear?** → Voice detection is working
- **✓ Does "v2 pipeline: waiting for STT final" appear?** → Turn is progressing
- **✓ Does "voice v2: STT final transcript ready" appear?** → STT completed successfully
- **✗ Where exactly does the log sequence stop?** → That tells us where the bug is

## Special Diagnostics

If you see `turn cancelled before transcription`, check for:
- `v2 pipeline: turn was cancelled before STT finalization` → turn token was cancelled
- `v2 pipeline: calling stt_handle.join()...` → STT is hanging here
- `v2 capture: STT stream closed; breaking` → capture closed early
- Frame count and RMS values → audio data actually flowing?

## Next Steps

Once you provide logs with this enhanced diagnostics, I will be able to:
1. Pinpoint exactly where the turn fails
2. Determine if it's audio capture, STT finalization, or LLM routing
3. Provide a targeted fix

**Please test and share the full log output!**
