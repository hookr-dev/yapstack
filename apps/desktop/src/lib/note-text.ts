/**
 * Strip Tiptap-produced HTML down to plain text suitable for embedding.
 *
 * Embeddings shouldn't see angle brackets, attribute noise, or entity
 * escapes — they bias the model away from the actual semantics. We use
 * the browser's parser when available (works in the Tauri webview and
 * in jsdom-backed tests); a regex fallback covers any environment that
 * lacks a DOM.
 *
 * Block-level elements get newline boundaries so a paragraph followed
 * by a list doesn't smash into one continuous run; inline elements
 * leave token boundaries intact.
 */
const BLOCK_SELECTOR =
  "address,article,aside,blockquote,br,dd,details,dialog,div,dl,dt,fieldset,figcaption,figure,footer,form,h1,h2,h3,h4,h5,h6,header,hgroup,hr,li,main,nav,ol,p,pre,section,table,td,th,tr,ul";

export function normalizeTiptapToText(html: string | null | undefined): string {
  if (!html) return "";
  const trimmed = html.trim();
  if (!trimmed) return "";

  if (typeof DOMParser !== "undefined") {
    const doc = new DOMParser().parseFromString(trimmed, "text/html");
    // Drop script/style — they should never be in Tiptap output but
    // defending against pasted-in content.
    doc.querySelectorAll("script,style").forEach((n) => n.remove());

    // Insert sentinel newlines after block-level closes so textContent
    // doesn't run paragraphs together.
    doc.querySelectorAll(BLOCK_SELECTOR).forEach((el) => {
      el.insertAdjacentText("afterend", "\n");
    });

    const raw = doc.body?.textContent ?? doc.documentElement?.textContent ?? "";
    return collapseWhitespace(raw);
  }

  // Fallback for non-DOM environments: regex strip + entity decode for
  // the common entities. Coverage is intentionally minimal — Tiptap's
  // output rarely contains anything beyond &amp;/&lt;/&gt;/&quot;/&#39;.
  const stripped = trimmed
    .replace(/<\s*\/?\s*(?:br|p|div|li|h[1-6])\s*[^>]*>/gi, "\n")
    .replace(/<[^>]+>/g, "")
    .replace(/&nbsp;/gi, " ")
    .replace(/&amp;/gi, "&")
    .replace(/&lt;/gi, "<")
    .replace(/&gt;/gi, ">")
    .replace(/&quot;/gi, '"')
    .replace(/&#39;/gi, "'");
  return collapseWhitespace(stripped);
}

function collapseWhitespace(s: string): string {
  // Preserve hard line breaks (so the embedder treats paragraphs as
  // separate sentences) but collapse runs of spaces and tabs and
  // trim leading/trailing whitespace per line + overall.
  return s
    .split(/\r?\n/)
    .map((line) => line.replace(/[\t ]+/g, " ").trim())
    .filter((line) => line.length > 0)
    .join("\n");
}
