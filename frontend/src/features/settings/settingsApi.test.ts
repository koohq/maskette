import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { analysisBackend, matchingPreset, SAME_AS_AGENT, URL_PRESETS } from "./settingsApi";

// `import.meta.url` is not a file URL under jsdom; Vitest gives the test file's folder as `import.meta.dirname`.
const STORE = join(import.meta.dirname, "../../../../src-tauri/src/backend/settings/store.rs");

describe("settings API helpers", () => {
  it("presets the backend's own default URLs", () => {
    const store = readFileSync(STORE, "utf8");
    const url = (id: string) => URL_PRESETS.find((preset) => preset.id === id)?.url;
    expect(store).toContain(`const OPENROUTER_BASE_URL: &str = "${url("openrouter")}";`);
    // The chat assistant reuses the Ark image key only on the Ark image URL.
    expect(store).toContain(`const ARK_BASE_URL: &str = "${url("ark")}";`);
    expect(store).toContain(`text("siliconflow_base_url", "${url("siliconflow")}")`);
    expect(store).toContain(`pub const SAME_AS_AGENT: &str = "${SAME_AS_AGENT}";`);
  });

  it("matches a preset whatever the trailing slash", () => {
    expect(matchingPreset(" https://openrouter.ai/api/v1/ ")?.id).toBe("openrouter");
    expect(matchingPreset("https://example.test/v1")).toBeUndefined();
    expect(matchingPreset(undefined)).toBeUndefined();
  });

  it("names the backend analysis runs on", () => {
    expect(analysisBackend({ llm_provider: SAME_AS_AGENT, agent_llm_provider: "claude_code" })).toBe("claude_code");
    expect(analysisBackend({ llm_provider: "codex", agent_llm_provider: "openai_compatible" })).toBe("codex");
  });
});
