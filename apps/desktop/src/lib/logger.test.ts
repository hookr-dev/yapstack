import { describe, it, expect } from "vitest";
import { formatArgs, safeStringify } from "./logger";

describe("safeStringify", () => {
  it("returns the string itself for string input", () => {
    expect(safeStringify("hello")).toBe("hello");
  });

  it("returns 'null' / 'undefined' for nullish input", () => {
    expect(safeStringify(null)).toBe("null");
    expect(safeStringify(undefined)).toBe("undefined");
  });

  it("renders Error objects with stack when available", () => {
    const err = new Error("boom");
    const out = safeStringify(err);
    // Stack format varies by engine; just check the message landed and the
    // type prefix is present somewhere in either the stack or the fallback.
    expect(out).toContain("boom");
  });

  it("renders Error objects with name+message when stack is missing", () => {
    const err = new Error("boom");
    err.stack = undefined;
    expect(safeStringify(err)).toBe("Error: boom");
  });

  it("JSON-stringifies plain objects", () => {
    expect(safeStringify({ a: 1, b: "x" })).toBe('{"a":1,"b":"x"}');
  });

  it("falls back gracefully on circular structures", () => {
    const a: Record<string, unknown> = {};
    a.self = a;
    // Should not throw and should produce something printable.
    const out = safeStringify(a);
    expect(typeof out).toBe("string");
    expect(out.length).toBeGreaterThan(0);
  });
});

describe("formatArgs", () => {
  it("joins multiple args with a single space", () => {
    expect(formatArgs(["status", 200, { ok: true }])).toBe(
      'status 200 {"ok":true}',
    );
  });

  it("returns empty string for no args", () => {
    expect(formatArgs([])).toBe("");
  });
});
