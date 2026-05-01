# Manual QA: device-change auto-failover

Hardware-driven scenarios that the unit-test harness can't cover. Run on macOS (Apple silicon, current OS) before shipping a release that touches the audio capture path.

Prerequisites:
- Bluetooth headset (AirPods or equivalent) paired and connectable.
- USB microphone available for hot-plug.
- Built-in mic + speakers usable as the fallback default.

> **Domain language**: terms like Capture, Mic, System audio, Source, Stream, Stream restart, Auto-failover, and Device broker are defined in [docs/UBIQUITOUS_LANGUAGE.md](../UBIQUITOUS_LANGUAGE.md).

---

## 1. AirPods mid-session drop (Mic, follow-default)

**Setup.** Pair AirPods. In System Settings → Sound, set them as the default input. In YapStack settings, leave the mic selection on "follow system default" (no explicit pick). Start a Capture and let it run for ~5 seconds.

**Action.** Power off the AirPods (or take them out and let macOS disconnect them).

**Expect.**
- Capture continues without complete loss of audio. A brief gap (≤500 ms) is acceptable.
- A single transient toast appears: "Switched mic to {built-in mic name}".
- Live transcription segments resume within a few seconds.
- No `restart_abandoned` toast.

## 2. USB mic mid-session unplug (Mic, follow-default)

**Setup.** Plug a USB mic. Confirm it became the system default input. Start a Capture.

**Action.** Unplug the USB mic.

**Expect.**
- Auto-failover toast names the new default (typically "MacBook Pro Microphone").
- Capture continues; segments keep flowing.

## 3. System Settings default-input change mid-session

**Setup.** With at least two input devices available, start a Capture (still on follow-default).

**Action.** Open System Settings → Sound → Input and pick a different device.

**Expect.**
- Toast names the device the user just picked.
- No restart loop / no spurious second toast within a second of the first.

## 4. Idle hot-plug visibility

**Setup.** Open YapStack's audio settings panel (or Onboarding's audio step) without any active Capture.

**Action.** Plug in a USB mic that wasn't connected when the app launched.

**Expect.**
- The device list reflects the new device within ~250 ms (broker's debounce window).
- No app restart needed. No manual refresh button needed.

## 5. System audio output change mid-session

**Setup.** Start Capture in `Mixed` or `SystemOnly` mode with the built-in speakers as default output.

**Action.** In Control Center (or System Settings → Sound → Output), switch the default output (e.g. to AirPods, then back).

**Expect.**
- A toast names the new system audio device on each change.
- The system-audio loopback Stream rebinds to the new output. System-audio segments resume after the brief gap.

## 6. Mixed-source unrecoverable failure (fail-fast)

**Setup.** Plug a USB mic. Start a `Mixed` Capture using that mic. Confirm both Mic and System audio are flowing.

**Action.** Force the Mic restart attempts to exhaust. Easiest reproduction: rapidly unplug and replug the USB mic several times in quick succession (faster than the broker can settle), so the restart counter reaches `STREAM_RESTART_MAX_ATTEMPTS` (3).

**Expect.**
- A `restart_abandoned` error toast appears for the Mic.
- Both Sources stop. The session ends through the normal stop path (segments drained, WAV finalized, Stopped phase emitted).
- No "limping along on system audio only" state.

## 7. Explicit non-default mic preserved when still alive

**Setup.** In settings, explicitly pick a non-default mic (e.g. the built-in mic when AirPods are the default). Start a Capture.

**Action.** Change the system default input via System Settings (without disconnecting the user's chosen mic).

**Expect.**
- **No** Auto-failover toast. The user's pick still works, so the broker leaves it alone.
- Capture continues uninterrupted on the explicitly chosen device.

## 8. Explicit pick disappears (force failover)

**Setup.** Plug a USB mic, explicitly pick it in settings, start a Capture.

**Action.** Unplug the USB mic.

**Expect.**
- The persisted `selectedMicDeviceId` is reconciled to "follow default" (visible in settings: the picker shows the new default).
- A "Selected microphone disappeared — using {default name}" toast appears (id `mic-disappeared`).
- The Capture rebinds to the new default and continues.

## 9. AirPods reconnect during preflight

**Setup.** Connect AirPods, set as default input, then disconnect — wait for macOS to fall back to the built-in. Start a new session immediately.

**Expect.**
- Preflight does not trigger a default-device-change restart (that path is gone). It only catches stream errors and write-pos stalls.
- The session starts cleanly on whichever device is currently default.

## 10. Concurrent device storm

**Setup.** Start a Capture. While it's running, perform: USB unplug → wait 1 s → AirPods connect → wait 1 s → AirPods disconnect.

**Expect.**
- The broker coalesces overlapping events within each 250 ms window. At most one toast per discrete user action.
- No `restart_abandoned` from over-eager retries.
- Capture is alive at the end on whichever device is current.
