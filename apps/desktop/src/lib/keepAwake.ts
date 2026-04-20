// Prevents macOS WKWebView from throttling this window's JS runtime when
// occluded (App Nap). A silent AudioContext source keeps the page in the
// "active" audible-media state, matching the trick used by Meet/Discord/etc.
// Dictation processing runs awaits while the user is typing into another
// app — without this, those awaits can pause until the main window refocuses.

let ctx: AudioContext | null = null;
let source: AudioBufferSourceNode | null = null;
let refCount = 0;

export function startKeepAwake() {
  refCount++;
  if (ctx) return;
  try {
    const AC = window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    if (!AC) return;
    ctx = new AC();
    const buffer = ctx.createBuffer(1, ctx.sampleRate, ctx.sampleRate);
    const gain = ctx.createGain();
    gain.gain.value = 0;
    source = ctx.createBufferSource();
    source.buffer = buffer;
    source.loop = true;
    source.connect(gain).connect(ctx.destination);
    source.start();
  } catch {
    ctx = null;
    source = null;
  }
}

export function stopKeepAwake() {
  if (refCount > 0) refCount--;
  if (refCount > 0) return;
  try {
    source?.stop();
    source?.disconnect();
  } catch {
    // ignore
  }
  source = null;
  ctx?.close().catch(() => {});
  ctx = null;
}
