import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  analysisBackend,
  matchingPreset,
  presetOfService,
  regionUrl,
  SAME_AS_AGENT,
  serviceOf,
  URL_PRESETS,
  urlPreset,
} from "./settingsApi";

// `import.meta.url` is not a file URL under jsdom; Vitest gives the test file's folder as `import.meta.dirname`.
const STORE = join(import.meta.dirname, "../../../../src-tauri/src/backend/settings/store.rs");

describe("settings API helpers", () => {
  it("presets the backend's own default URLs", () => {
    const store = readFileSync(STORE, "utf8");
    const url = (id: string) => URL_PRESETS.find((preset) => preset.id === id)?.url;
    expect(store).toContain(`const OPENROUTER_BASE_URL: &str = "${url("openrouter")}";`);
    // The chat assistant on Ark's URL shares the Ark image key.
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

  // The same cases as `a_service_is_the_host_of_its_url` in `src-tauri/src/backend/settings/tests.rs`.
  it("saves a key under the host of its URL", () => {
    const cases: [string, string][] = [
      ["https://openrouter.ai/api/v1", "openrouter.ai"],
      [" https://OpenRouter.ai/api/v1/ ", "openrouter.ai"],
      ["https://ark.cn-beijing.volces.com/api/plan/v3", "ark.cn-beijing.volces.com"],
      ["https://ark.ap-southeast.bytepluses.com/api/v3", "ark.ap-southeast.bytepluses.com"],
      ["http://localhost:11434/v1", "localhost:11434"],
      ["https://api.example.test:443/v1", "api.example.test"],
      ["http://[::1]:8080/v1", "[::1]:8080"],
      ["openrouter.ai/api/v1/", "openrouter.ai/api/v1"],
      ["localhost:8080/v1", "localhost:8080/v1"],
      ["", ""],
    ];
    for (const [url, service] of cases) expect(serviceOf(url), url).toBe(service);
  });

  it("knows the services of the presets and their sites", () => {
    expect(presetOfService("openrouter.ai")).toEqual({ preset: urlPreset("openrouter") });
    expect(presetOfService("ark.cn-beijing.volces.com")).toEqual({ preset: urlPreset("ark"), region: "mainland" });
    expect(presetOfService("api.siliconflow.com")).toEqual({
      preset: urlPreset("siliconflow"),
      region: "international",
    });
    expect(presetOfService("llm.example.test")).toBeUndefined();
  });

  it("names the backend analysis runs on", () => {
    expect(analysisBackend({ llm_provider: SAME_AS_AGENT, agent_llm_provider: "claude_code" })).toBe("claude_code");
    expect(analysisBackend({ llm_provider: "codex", agent_llm_provider: "openai_compatible" })).toBe("codex");
  });
});
