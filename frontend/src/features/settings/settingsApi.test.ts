import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { analysisBackend, matchingPreset, regionUrl, SAME_AS_AGENT, URL_PRESETS, urlPreset } from "./settingsApi";

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
    expect(matchingPreset(" https://openrouter.ai/api/v1/ ")).toEqual({ preset: urlPreset("openrouter") });
    expect(matchingPreset("https://example.test/v1")).toBeUndefined();
    expect(matchingPreset(undefined)).toBeUndefined();
  });

  it("tells the mainland China and international sites of a service apart", () => {
    const siliconflow = urlPreset("siliconflow");
    expect(matchingPreset("https://api.siliconflow.cn/v1")).toEqual({ preset: siliconflow, region: "mainland" });
    expect(matchingPreset("https://api.siliconflow.com/v1/")).toEqual({ preset: siliconflow, region: "international" });
    expect(matchingPreset("https://ark.ap-southeast.bytepluses.com/api/v3")).toEqual({
      preset: urlPreset("ark"),
      region: "international",
    });
    // An image backend's URL field looks only at its own service.
    expect(matchingPreset("https://api.siliconflow.com/v1", [urlPreset("ark")])).toBeUndefined();

    expect(regionUrl(siliconflow, "international")).toBe("https://api.siliconflow.com/v1");
    expect(regionUrl(siliconflow, "mainland")).toBe("https://api.siliconflow.cn/v1");
    expect(regionUrl(urlPreset("openai"), "international")).toBe("https://api.openai.com/v1");
  });

  it("names the backend analysis runs on", () => {
    expect(analysisBackend({ llm_provider: SAME_AS_AGENT, agent_llm_provider: "claude_code" })).toBe("claude_code");
    expect(analysisBackend({ llm_provider: "codex", agent_llm_provider: "openai_compatible" })).toBe("codex");
  });
});
