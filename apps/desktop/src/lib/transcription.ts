const MIN_VOCAB_NAME_LENGTH = 4;
const MAX_VOCAB_CHARS = 80;

export function buildVocabularyHints(
  folders: { name: string }[],
  tags: { name: string }[],
): string | null {
  const parts: string[] = [];
  for (const folder of folders) {
    if (folder.name.length >= MIN_VOCAB_NAME_LENGTH && !parts.includes(folder.name)) {
      parts.push(folder.name);
    }
  }
  for (const tag of tags) {
    if (tag.name.length >= MIN_VOCAB_NAME_LENGTH && !parts.includes(tag.name)) {
      parts.push(tag.name);
    }
  }
  if (parts.length === 0) return null;
  let combined = parts.join(", ");
  if (combined.length > MAX_VOCAB_CHARS) {
    combined = combined.slice(0, MAX_VOCAB_CHARS);
    const lastComma = combined.lastIndexOf(",");
    if (lastComma > 0) combined = combined.slice(0, lastComma);
  }
  return combined;
}
