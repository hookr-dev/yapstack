import { describe, it, expect } from "vitest";
import {
  buildInsight,
  shouldShowInsightOverlay,
  resolveTriggerConfig,
  evaluateTrigger,
  describeTrigger,
  applyTemplate,
  TRIGGER_PRESETS,
  INSIGHT_TEMPLATES,
  type Insight,
  type InsightTrigger,
  type TriggerConfig,
} from "./insights";

function insight(overrides: Partial<Insight> = {}): Insight {
  return { ...buildInsight("Test"), ...overrides };
}

function trigger(overrides: Partial<InsightTrigger> = {}): InsightTrigger {
  return { preset: "custom", ...TRIGGER_PRESETS.balanced, ...overrides };
}

function config(overrides: Partial<TriggerConfig> = {}): TriggerConfig {
  return { ...TRIGGER_PRESETS.balanced, ...overrides };
}

describe("shouldShowInsightOverlay", () => {
  const slot = insight({ id: "a" });

  it("shows when feature on, session live, and the Current Insight exists", () => {
    expect(
      shouldShowInsightOverlay({
        enabled: true,
        liveTranscriptionActive: true,
        currentInsightId: "a",
        slots: [slot],
      }),
    ).toBe(true);
  });

  it("hides when the feature is off", () => {
    expect(
      shouldShowInsightOverlay({
        enabled: false,
        liveTranscriptionActive: true,
        currentInsightId: "a",
        slots: [slot],
      }),
    ).toBe(false);
  });

  it("hides when no session is live", () => {
    expect(
      shouldShowInsightOverlay({
        enabled: true,
        liveTranscriptionActive: false,
        currentInsightId: "a",
        slots: [slot],
      }),
    ).toBe(false);
  });

  it("hides when there is no Current Insight", () => {
    expect(
      shouldShowInsightOverlay({
        enabled: true,
        liveTranscriptionActive: true,
        currentInsightId: null,
        slots: [slot],
      }),
    ).toBe(false);
  });

  // Regression: deleting the running Insight mid-session leaves a dangling
  // currentInsightId; the gate must hide rather than keep a stale overlay.
  it("hides when the Current Insight no longer exists in slots", () => {
    expect(
      shouldShowInsightOverlay({
        enabled: true,
        liveTranscriptionActive: true,
        currentInsightId: "gone",
        slots: [slot],
      }),
    ).toBe(false);
  });
});

describe("resolveTriggerConfig", () => {
  it("returns the PRESETS tuple for a named preset, ignoring stored numbers", () => {
    // Stored numbers diverge from the preset; the preset must win.
    const i = insight({
      trigger: trigger({
        preset: "responsive",
        thresholdWords: 999,
        settleSeconds: 999,
        minIntervalSeconds: 999,
        maxWaitSeconds: 999,
      }),
    });
    expect(resolveTriggerConfig(i)).toEqual(TRIGGER_PRESETS.responsive);
  });

  it("returns the stored values for a custom preset", () => {
    const custom = {
      thresholdWords: 25,
      settleSeconds: 2,
      minIntervalSeconds: 5,
      maxWaitSeconds: 40,
    };
    const i = insight({ trigger: { preset: "custom", ...custom } });
    expect(resolveTriggerConfig(i)).toEqual(custom);
  });

  it("sanitizes NaN / negative custom values", () => {
    const i = insight({
      trigger: {
        preset: "custom",
        thresholdWords: -5,
        settleSeconds: Number.NaN,
        minIntervalSeconds: -1,
        maxWaitSeconds: Number.NaN,
      },
    });
    expect(resolveTriggerConfig(i)).toEqual({
      thresholdWords: 0,
      settleSeconds: 0,
      minIntervalSeconds: 0,
      maxWaitSeconds: TRIGGER_PRESETS.balanced.maxWaitSeconds,
    });
  });
});

describe("applyTemplate", () => {
  it("seeds the prompt + recommended cadence for a type", () => {
    const applied = applyTemplate("glossary");
    expect(applied.type).toBe("glossary");
    expect(applied.prompt).toBe(INSIGHT_TEMPLATES.glossary.prompt);
    // Glossary recommends the responsive cadence.
    expect(applied.trigger).toEqual({
      preset: "responsive",
      ...TRIGGER_PRESETS.responsive,
    });
  });

  it("rolling summary recommends the relaxed cadence", () => {
    expect(applyTemplate("summary").trigger.preset).toBe("relaxed");
  });

  it("every template references a real cadence preset and a non-empty prompt", () => {
    for (const t of Object.values(INSIGHT_TEMPLATES)) {
      expect(t.prompt.length).toBeGreaterThan(0);
      expect(TRIGGER_PRESETS[t.cadence]).toBeDefined();
    }
  });
});

describe("evaluateTrigger", () => {
  const cfg = config({
    thresholdWords: 100,
    settleSeconds: 3,
    minIntervalSeconds: 20,
    maxWaitSeconds: 90,
  });

  it("does not fire when there is no new content", () => {
    expect(
      evaluateTrigger({
        newWords: 0,
        sinceFireMs: 999_000,
        sincePauseMs: 999_000,
        hasNewContent: false,
        config: cfg,
      }),
    ).toEqual({ fire: false, reason: "no-content" });
  });

  it("floor blocks even when threshold, settle and ceiling would all fire", () => {
    expect(
      evaluateTrigger({
        newWords: 500,
        sinceFireMs: 19_000, // < F (20s)
        sincePauseMs: 999_000,
        hasNewContent: true,
        config: cfg,
      }),
    ).toEqual({ fire: false, reason: "floor" });
  });

  it("ceiling forces a fire even when threshold unmet and not settled", () => {
    expect(
      evaluateTrigger({
        newWords: 1, // threshold unmet
        sinceFireMs: 90_000, // == M
        sincePauseMs: 0, // not settled
        hasNewContent: true,
        config: cfg,
      }),
    ).toEqual({ fire: true, reason: "ceiling" });
  });

  it("fires on threshold + settle (below the ceiling)", () => {
    expect(
      evaluateTrigger({
        newWords: 100, // == W
        sinceFireMs: 30_000, // > F, < M
        sincePauseMs: 3_000, // == P
        hasNewContent: true,
        config: cfg,
      }),
    ).toEqual({ fire: true, reason: "threshold-settle" });
  });

  it("waits when threshold met but not settled", () => {
    expect(
      evaluateTrigger({
        newWords: 200,
        sinceFireMs: 30_000,
        sincePauseMs: 1_000, // < P
        hasNewContent: true,
        config: cfg,
      }),
    ).toEqual({ fire: false, reason: "waiting" });
  });

  it("waits when settled but threshold unmet (below ceiling)", () => {
    expect(
      evaluateTrigger({
        newWords: 10,
        sinceFireMs: 30_000,
        sincePauseMs: 10_000,
        hasNewContent: true,
        config: cfg,
      }),
    ).toEqual({ fire: false, reason: "waiting" });
  });

  it("boundary: sinceFire === F is not floor-blocked; === M is ceiling", () => {
    const atFloor = evaluateTrigger({
      newWords: 0,
      sinceFireMs: 20_000, // == F (not < F)
      sincePauseMs: 0,
      hasNewContent: true,
      config: cfg,
    });
    // At exactly F the floor no longer blocks; threshold unmet + not settled +
    // below ceiling ⇒ waiting (not floor).
    expect(atFloor).toEqual({ fire: false, reason: "waiting" });
  });

  // Degenerate Interval preset: W=0, F===M ⇒ fires exactly on the heartbeat.
  describe("degenerate interval {W:0, F:30, M:30}", () => {
    const interval = config({
      thresholdWords: 0,
      settleSeconds: 0,
      minIntervalSeconds: 30,
      maxWaitSeconds: 30,
    });
    it("does not fire at 29s (floor)", () => {
      expect(
        evaluateTrigger({
          newWords: 5,
          sinceFireMs: 29_000,
          sincePauseMs: 0,
          hasNewContent: true,
          config: interval,
        }),
      ).toEqual({ fire: false, reason: "floor" });
    });
    it("fires at 30s (ceiling)", () => {
      expect(
        evaluateTrigger({
          newWords: 5,
          sinceFireMs: 30_000,
          sincePauseMs: 0,
          hasNewContent: true,
          config: interval,
        }),
      ).toEqual({ fire: true, reason: "ceiling" });
    });
  });

  // Degenerate Accumulator: P=0, large M ⇒ fires on threshold before ceiling.
  it("accumulator (P:0) fires on threshold before the ceiling", () => {
    const accumulator = config({
      thresholdWords: 40,
      settleSeconds: 0,
      minIntervalSeconds: 8,
      maxWaitSeconds: 600,
    });
    expect(
      evaluateTrigger({
        newWords: 40,
        sinceFireMs: 10_000, // > F, well below M
        sincePauseMs: 0, // settled trivially (P=0)
        hasNewContent: true,
        config: accumulator,
      }),
    ).toEqual({ fire: true, reason: "threshold-settle" });
  });
});

describe("describeTrigger", () => {
  it("describes the balanced cadence (threshold AND settle + bounds)", () => {
    expect(describeTrigger(TRIGGER_PRESETS.balanced)).toBe(
      "Updates after ~90 new words and a 1.5s pause — no sooner than every 15s, at least every 60s.",
    );
  });

  it("renders fractional settle seconds (responsive)", () => {
    expect(describeTrigger(TRIGGER_PRESETS.responsive)).toBe(
      "Updates after ~40 new words and a 0.8s pause — no sooner than every 8s, at least every 30s.",
    );
  });

  it("collapses a plain interval (W:0, F===M) to 'every Ns'", () => {
    expect(
      describeTrigger(
        config({
          thresholdWords: 0,
          settleSeconds: 0,
          minIntervalSeconds: 30,
          maxWaitSeconds: 30,
        }),
      ),
    ).toBe("Updates every 30s.");
  });

  it("omits the settle clause when P is 0 (accumulator)", () => {
    expect(
      describeTrigger(
        config({
          thresholdWords: 40,
          settleSeconds: 0,
          minIntervalSeconds: 8,
          maxWaitSeconds: 45,
        }),
      ),
    ).toBe(
      "Updates after ~40 new words — no sooner than every 8s, at least every 45s.",
    );
  });

  it("omits the floor clause when F is 0", () => {
    expect(
      describeTrigger(
        config({
          thresholdWords: 120,
          settleSeconds: 3,
          minIntervalSeconds: 0,
          maxWaitSeconds: 90,
        }),
      ),
    ).toBe("Updates after ~120 new words and a 3s pause — at least every 90s.");
  });
});
