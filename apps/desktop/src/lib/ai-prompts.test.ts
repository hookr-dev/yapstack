import { describe, it, expect } from "vitest";
import {
  getSystemPrompt,
  getSystemPromptWithToolContext,
  getMultiSessionSystemPrompt,
  GENERAL_DIRECTIVE,
} from "./ai-prompts";
import { ACTIONS } from "./ai-actions";

const summarizeDirective = ACTIONS.find((a) => a.id === "summarize")!.directive;
const keyPointsDirective = ACTIONS.find((a) => a.id === "key-points")!.directive;

describe("getSystemPrompt", () => {
  it("includes directive for summarize action", () => {
    const prompt = getSystemPrompt(summarizeDirective, "", "", []);
    expect(prompt).toContain("note-taker");
  });

  it("includes directive for key-points action", () => {
    const prompt = getSystemPrompt(keyPointsDirective, "", "", []);
    expect(prompt).toContain("key-points extractor");
  });

  it("includes transcript when provided", () => {
    const prompt = getSystemPrompt(GENERAL_DIRECTIVE, "Hello world", "", []);
    expect(prompt).toContain("Session Transcript");
    expect(prompt).toContain("Hello world");
  });

  it("includes notes when provided", () => {
    const prompt = getSystemPrompt(GENERAL_DIRECTIVE, "", "Some notes", []);
    expect(prompt).toContain("Notes");
    expect(prompt).toContain("Some notes");
  });

  it("includes attachments", () => {
    const prompt = getSystemPrompt(GENERAL_DIRECTIVE, "", "", [
      { name: "file.txt", content: "file content" },
    ]);
    expect(prompt).toContain("Attached Files");
    expect(prompt).toContain("file.txt");
    expect(prompt).toContain("file content");
  });

  it("omits sections with empty content", () => {
    const prompt = getSystemPrompt(GENERAL_DIRECTIVE, "", "", []);
    expect(prompt).not.toContain("Session Transcript");
    expect(prompt).not.toContain("## Notes");
  });

  it("includes citation instruction when transcript is provided", () => {
    const prompt = getSystemPrompt(GENERAL_DIRECTIVE, "some transcript", "", []);
    expect(prompt).toContain("[[seg:ID]]");
  });

  it("omits citation instruction when transcript is empty", () => {
    const prompt = getSystemPrompt(GENERAL_DIRECTIVE, "", "", []);
    expect(prompt).not.toContain("[[seg:ID]]");
  });

  it("always includes notes guidance regardless of transcript", () => {
    const withTranscript = getSystemPrompt(GENERAL_DIRECTIVE, "text", "", []);
    const withoutTranscript = getSystemPrompt(GENERAL_DIRECTIVE, "", "", []);
    expect(withTranscript).toContain("saving to notes");
    expect(withoutTranscript).toContain("saving to notes");
  });

  it("uses a custom directive string directly", () => {
    const prompt = getSystemPrompt("You are a custom analyzer.", "", "", []);
    expect(prompt).toContain("You are a custom analyzer.");
  });
});

describe("getSystemPromptWithToolContext", () => {
  it("includes session metadata", () => {
    const prompt = getSystemPromptWithToolContext(GENERAL_DIRECTIVE, "", "", [], {
      title: "My Session",
      isPinned: false,
      hasNotes: true,
    });
    expect(prompt).toContain('Current title: "My Session"');
    expect(prompt).toContain("Pinned: no");
    expect(prompt).toContain("Has existing notes: yes");
  });

  it("shows pinned yes when isPinned", () => {
    const prompt = getSystemPromptWithToolContext(GENERAL_DIRECTIVE, "", "", [], {
      title: "Test",
      isPinned: true,
      hasNotes: false,
    });
    expect(prompt).toContain("Pinned: yes");
    expect(prompt).toContain("Has existing notes: no");
  });

  it("includes base prompt content", () => {
    const prompt = getSystemPromptWithToolContext(
      GENERAL_DIRECTIVE,
      "transcript text",
      "",
      [],
      { title: "T", isPinned: false, hasNotes: false },
    );
    expect(prompt).toContain("transcript text");
  });
});

describe("getMultiSessionSystemPrompt", () => {
  it("includes sessions context", () => {
    const prompt = getMultiSessionSystemPrompt("session data here", []);
    expect(prompt).toContain("session data here");
    expect(prompt).toContain("Sessions");
  });

  it("includes single folder context layer", () => {
    const prompt = getMultiSessionSystemPrompt("data", [], [
      { name: "Work", description: "Work meetings" },
    ]);
    expect(prompt).toContain("Organizational context:");
    expect(prompt).toContain("**Work:** Work meetings");
  });

  it("includes multiple folder context layers in order", () => {
    const prompt = getMultiSessionSystemPrompt("data", [], [
      { name: "Company", description: "All company sessions" },
      { name: "Engineering", description: "Engineering team meetings" },
    ]);
    expect(prompt).toContain("Organizational context:");
    expect(prompt).toContain("**Company:** All company sessions");
    expect(prompt).toContain("**Engineering:** Engineering team meetings");
    // Root should appear before child
    const companyIdx = prompt.indexOf("**Company:**");
    const engIdx = prompt.indexOf("**Engineering:**");
    expect(companyIdx).toBeLessThan(engIdx);
  });

  it("omits organizational context when array is empty", () => {
    const prompt = getMultiSessionSystemPrompt("data", [], []);
    expect(prompt).not.toContain("Organizational context:");
  });

  it("omits organizational context when not provided", () => {
    const prompt = getMultiSessionSystemPrompt("data", []);
    expect(prompt).not.toContain("Organizational context:");
  });

  it("includes attachments", () => {
    const prompt = getMultiSessionSystemPrompt("data", [
      { name: "doc.md", content: "# Doc" },
    ]);
    expect(prompt).toContain("Attached Files");
    expect(prompt).toContain("doc.md");
  });
});
