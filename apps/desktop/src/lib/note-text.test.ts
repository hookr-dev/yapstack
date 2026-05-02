import { describe, it, expect } from "vitest";
import { normalizeTiptapToText } from "./note-text";

describe("normalizeTiptapToText", () => {
  it("returns empty for null/undefined/whitespace", () => {
    expect(normalizeTiptapToText(null)).toBe("");
    expect(normalizeTiptapToText(undefined)).toBe("");
    expect(normalizeTiptapToText("")).toBe("");
    expect(normalizeTiptapToText("   \n\n  ")).toBe("");
  });

  it("strips simple paragraph tags", () => {
    expect(normalizeTiptapToText("<p>hello world</p>")).toBe("hello world");
  });

  it("inserts boundaries between block elements", () => {
    const html = "<p>first paragraph</p><p>second paragraph</p>";
    const result = normalizeTiptapToText(html);
    expect(result).toContain("first paragraph");
    expect(result).toContain("second paragraph");
    // Must not be smashed together
    expect(result).not.toMatch(/paragraphsecond/);
  });

  it("flattens lists", () => {
    const html = "<ul><li>alpha</li><li>beta</li><li>gamma</li></ul>";
    const result = normalizeTiptapToText(html);
    expect(result).toContain("alpha");
    expect(result).toContain("beta");
    expect(result).toContain("gamma");
    expect(result).not.toMatch(/alphabetagamma/);
  });

  it("preserves link text but strips href attributes", () => {
    const html = '<p>see <a href="https://example.com">the docs</a></p>';
    expect(normalizeTiptapToText(html)).toBe("see the docs");
  });

  it("decodes entities", () => {
    expect(normalizeTiptapToText("<p>a &amp; b</p>")).toBe("a & b");
    expect(normalizeTiptapToText("<p>&quot;hi&quot;</p>")).toBe('"hi"');
  });

  it("strips script and style content defensively", () => {
    const html =
      '<p>safe</p><script>alert("evil")</script><style>p{color:red}</style>';
    const result = normalizeTiptapToText(html);
    expect(result).toContain("safe");
    expect(result).not.toContain("alert");
    expect(result).not.toContain("color:red");
  });

  it("collapses internal whitespace per line", () => {
    expect(normalizeTiptapToText("<p>too    many   spaces</p>")).toBe(
      "too many spaces",
    );
  });

  it("handles headings and code together", () => {
    const html =
      "<h1>Title</h1><p>intro <code>foo()</code> outro</p><pre>raw block</pre>";
    const result = normalizeTiptapToText(html);
    expect(result).toContain("Title");
    expect(result).toContain("intro foo() outro");
    expect(result).toContain("raw block");
  });
});
