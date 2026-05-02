import { commands, type SourceKindDto, type MissingRowDto } from "@/lib/tauri";
import { useAppStore } from "@/stores/appStore";
import { normalizeTiptapToText } from "@/lib/note-text";

// Bounded background loop that catches rows from before the feature
// shipped plus any newly-inserted row whose live fire-and-forget embed
// didn't land. Pauses while recording or while embeddings are disabled.
//
// Language gating uses the global Settings.language since we don't
// persist per-row creation language. An English user with historical
// non-English content will get those rows embedded with the English
// model (low-quality vectors, harmless); a non-English user pauses
// backfill entirely. A future migration can add per-row language.

const POLL_INTERVAL_MS = 5_000;
const BATCH_SIZE = 32;
const INTER_BATCH_DELAY_MS = 250;

const SURFACES: SourceKindDto[] = ["Segment", "Dictation", "Note"];

let interval: ReturnType<typeof setInterval> | null = null;
let inFlight = false;
let modelReadyCached = false;

async function modelReady(): Promise<boolean> {
  if (modelReadyCached) return true;
  const result = await commands.embeddingModelStatus();
  if (result.status === "ok" && result.data.ready) {
    modelReadyCached = true;
    return true;
  }
  return false;
}

function isPaused(): boolean {
  const { settings, activeSessionId } = useAppStore.getState();
  return (
    !settings.embeddingsEnabled ||
    settings.language !== "en" ||
    activeSessionId !== null
  );
}

function normalizeRowsIfNotes(
  kind: SourceKindDto,
  raw: MissingRowDto[],
): MissingRowDto[] {
  if (kind !== "Note") return raw;
  return raw
    .map((r) => ({ id: r.id, text: normalizeTiptapToText(r.text) }))
    .filter((r) => r.text.length > 0);
}

async function processSurface(kind: SourceKindDto): Promise<number> {
  const list = await commands.listMissingEmbeddings(kind, BATCH_SIZE);
  if (list.status !== "ok" || list.data.length === 0) return 0;
  const rows = normalizeRowsIfNotes(kind, list.data);
  if (rows.length === 0) return 0;
  const result = await commands.embedAndStoreBatch(kind, rows);
  if (result.status !== "ok") return 0;
  return result.data;
}

async function tick() {
  if (inFlight || isPaused()) return;
  if (!(await modelReady())) return;

  inFlight = true;
  try {
    for (const surface of SURFACES) {
      const written = await processSurface(surface);
      if (isPaused()) break;
      if (written > 0) {
        await new Promise((r) => setTimeout(r, INTER_BATCH_DELAY_MS));
      }
    }
  } catch (err) {
    console.warn("embedding-backfill tick failed:", err);
  } finally {
    inFlight = false;
  }
}

export function startEmbeddingBackfill(): () => void {
  if (interval !== null) return stopEmbeddingBackfill;
  void tick();
  interval = setInterval(() => {
    void tick();
  }, POLL_INTERVAL_MS);
  return stopEmbeddingBackfill;
}

export function stopEmbeddingBackfill() {
  if (interval !== null) {
    clearInterval(interval);
    interval = null;
  }
}
