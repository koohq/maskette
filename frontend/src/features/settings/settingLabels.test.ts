import { readFileSync } from "node:fs";
import { join } from "node:path";
import { afterAll, describe, expect, it } from "vitest";
import i18n from "../../i18n";
import { SETTING_KEYS, settingKind, settingLabel, settingLabels } from "./settingLabels";

// `import.meta.url` is not a file URL under jsdom; Vitest gives the test file's folder as `import.meta.dirname`.
const STORE = join(import.meta.dirname, "../../../../src-tauri/src/backend/settings/store.rs");

/** The part of `store.rs` from `start` to the end of the list it begins. */
function list(source: string, start: string): string {
  const from = source.indexOf(start);
  return source.slice(from, source.indexOf("];", from));
}

/**
 * The keys of the editable settings (`FIELDS`), built with `text(...)` or `choice(...)`, and of the fields that show a
 * role's API key (`KEY_FIELDS`).
 */
function backendSettingKeys(): string[] {
  const source = readFileSync(STORE, "utf8");
  const fields = list(source, "pub const FIELDS");
  const keyFields = list(source, "pub const KEY_FIELDS");
  return [
    ...[...fields.matchAll(/\b(?:text|choice)\(\s*"([a-z][a-z0-9_]*)"/g)].flatMap((match) => match[1] ?? []),
    ...[...keyFields.matchAll(/"([a-z][a-z0-9_]*)"/g)].flatMap((match) => match[1] ?? []),
  ];
}

afterAll(async () => {
  await i18n.changeLanguage("zh-CN");
});

describe("setting labels", () => {
  it("knows where every editable setting sits on the settings page", () => {
    const keys = backendSettingKeys();
    // The scan must find the settings it is meant to find, or it proves nothing.
    expect(keys).toContain("agent_llm_provider");
    expect(keys).toContain("openrouter_api_key");
    expect([...keys].sort()).toEqual([...SETTING_KEYS].sort());
  });

  it("names a setting by its section and field", async () => {
    await i18n.changeLanguage("ja");
    expect(settingLabel(i18n.t, "agent_llm_api_key")).toBe("「チャットアシスタント」の「API キー」");
    expect(settingLabels(i18n.t, "agent_llm_base_url, agent_llm_model")).toBe(
      "「チャットアシスタント」の「API URL」、「チャットアシスタント」の「モデル」",
    );
    await i18n.changeLanguage("en");
    expect(settingLabel(i18n.t, "openrouter_api_key")).toBe("API key (Image backend)");
    await i18n.changeLanguage("zh-CN");
    expect(settingLabel(i18n.t, "analysis_llm_model")).toBe("“参考图分析 LLM”中的“模型”");
  });

  it("keeps a key it does not know", async () => {
    await i18n.changeLanguage("ja");
    expect(settingLabel(i18n.t, "some_future_key")).toBe("some_future_key");
  });

  it("tells what kind of value the backend checks", () => {
    expect(settingKind("openrouter_extra_body")).toBe("extra_body");
    expect(settingKind("agent_llm_base_url")).toBe("other");
  });
});
