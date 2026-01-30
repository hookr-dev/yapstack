import { describe, it, expect } from "vitest";
import { ACTIONS, getAction, getActionIcon } from "./ai-actions";

describe("getAction", () => {
  it("returns definition for valid ID", () => {
    const action = getAction("summarize");
    expect(action).toBeDefined();
    expect(action!.label).toBe("Summarize");
  });

  it("has all 4 actions defined", () => {
    expect(ACTIONS).toHaveLength(4);
    const ids = ACTIONS.map((a) => a.id);
    expect(ids).toContain("summarize");
    expect(ids).toContain("key-points");
    expect(ids).toContain("action-items");
    expect(ids).toContain("meeting-minutes");
  });

  it("every action has a non-empty directive", () => {
    for (const action of ACTIONS) {
      expect(action.directive, `${action.id} should have a directive`).toBeTruthy();
      expect(action.directive.length).toBeGreaterThan(10);
    }
  });

  it("returns undefined for unknown ID", () => {
    expect(getAction("nonexistent")).toBeUndefined();
  });
});

describe("getActionIcon", () => {
  it("returns a component for valid ID", () => {
    const icon = getActionIcon("summarize");
    expect(icon).toBeDefined();
  });

  it("returns undefined for unknown ID", () => {
    expect(getActionIcon("nonexistent")).toBeUndefined();
  });
});
