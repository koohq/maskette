import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { afterAll, describe, expect, it } from "vitest";
import i18n from "../../i18n";
import { AgentApiError } from "./agentApi";
import { apiErrorText, DETAIL_CODES, detailText, errorDetail } from "./apiErrorText";

const LANGUAGES = ["ja", "en", "zh-CN"];

/**
 * Codes the backend sends that the screen never shows as text, so they stay untranslated: the screen always sends
 * these forms correctly (an action, the reference ids as JSON), or the URL is loaded by `<img>` or a download link.
 */
const NOT_SHOWN = new Set([
  "unknown_action", // the approve buttons only send known actions
  "reference_ids_invalid", // the editor always sends a JSON array
  "version_not_found", // the editor's image and the download link load these URLs directly
  "version_file_missing",
  "version_not_encoded",
  "project_not_exported", // export is a download link
]);

// `import.meta.url` is not a file URL under jsdom; Vitest gives the test file's folder as `import.meta.dirname`.
const BACKEND = join(import.meta.dirname, "../../../../src-tauri/src/backend");

/** Files whose plain `Err("...")` values become the `detail` of an HTTP error (they say so in their docs). */
const PLAIN_ERR_FILES = new Set(["vkp.rs", "history.rs"]);

function rustFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return rustFiles(path);
    return entry.name.endsWith(".rs") && entry.name !== "tests.rs" ? [path] : [];
  });
}

/** Every snake_case code the backend puts in an HTTP error's `detail` (an `ApiError`), with or without a value. */
function backendDetailCodes(): Set<string> {
  const codes = new Set<string>();
  for (const path of rustFiles(BACKEND)) {
    const source = readFileSync(path, "utf8").split("#[cfg(test)]")[0] ?? "";
    const plainErr = PLAIN_ERR_FILES.has(path.split(/[\\/]/).pop() ?? "");
    // String constants (`const UNSUPPORTED_TYPE: &str = "..."`), counted when an error line uses them.
    const constants = new Map(
      [...source.matchAll(/const ([A-Z][A-Z0-9_]*): &str = "([a-z][a-z0-9]*(?:_[a-z0-9]+)+)";/g)].map(
        (match) => [match[1], match[2]] as const,
      ),
    );
    for (const line of source.split("\n")) {
      const errorLine =
        /ApiError|bad_request\(|conflict\(|not_found\(|unprocessable\(/.test(line) ||
        // In those files a code also stands alone as the last value of a `map_err` closure.
        (plainErr && /Err\(|map_err\(|ok_or\(|let nothing|^\s*"[a-z0-9_]+"\s*$/.test(line));
      if (!errorLine) continue;
      for (const match of line.matchAll(/"([a-z][a-z0-9]*(?:_[a-z0-9]+)+)(?::[^"]*)?"/g)) {
        if (match[1]) codes.add(match[1]);
      }
      for (const [name, code] of constants) {
        if (new RegExp(`\\b${name}\\b`).test(line) && code) codes.add(code);
      }
    }
  }
  return codes;
}

afterAll(async () => {
  await i18n.changeLanguage("zh-CN");
});

describe("HTTP error codes", () => {
  it("translates or knowingly skips every code the backend sends", () => {
    const sent = backendDetailCodes();
    // The scan must find the codes it is meant to find, or it proves nothing.
    expect(sent).toContain("image_too_large");
    expect(sent).toContain("settings_not_saved");
    expect(sent).toContain("invalid_setting");
    expect(sent).toContain("not_a_zip");
    expect(sent).toContain("nothing_to_regenerate");
    const missing = [...sent].filter((code) => !DETAIL_CODES.has(code) && !NOT_SHOWN.has(code));
    expect(missing).toEqual([]);
    const stale = [...DETAIL_CODES, ...NOT_SHOWN].filter((code) => !sent.has(code));
    expect(stale).toEqual([]);
  });

  it("has every code in every language", async () => {
    for (const language of LANGUAGES) {
      await i18n.changeLanguage(language);
      for (const code of DETAIL_CODES) {
        const key = `agent.errors.${code}`;
        expect(i18n.exists(key, { lng: language, fallbackLng: false }), `${language} ${key}`).toBe(true);
      }
    }
  });

  it("shows a code in the UI language instead of the code", async () => {
    await i18n.changeLanguage("ja");
    expect(apiErrorText(i18n.t, new AgentApiError("image_too_large", 400))).toBe(
      "画像が大きすぎます（上限 20 MB）。小さくしてから、もう一度試してください。",
    );
    await i18n.changeLanguage("en");
    expect(apiErrorText(i18n.t, new AgentApiError("edit_note_required", 400))).toBe(
      "Describe what to change (the instruction is empty).",
    );
    await i18n.changeLanguage("zh-CN");
    expect(apiErrorText(i18n.t, new AgentApiError("unsupported_image_type", 400))).toBe(
      "不支持这种图片格式。请选择 PNG、JPEG 或 WebP 图片。",
    );
  });

  it("fills in the value a code carries", async () => {
    await i18n.changeLanguage("ja");
    expect(detailText(i18n.t, "invalid_setting:agent_extra_body")).toContain("設定 agent_extra_body の値");
    expect(detailText(i18n.t, "settings_not_saved: credential store: test_key: denied")).toBe(
      "設定を保存できませんでした。詳細：credential store: test_key: denied",
    );
    await i18n.changeLanguage("en");
    expect(detailText(i18n.t, "version_not_saved: disk full")).toBe(
      "The edited image could not be saved. Check the free disk space and write permissions. Details: disk full",
    );
    expect(detailText(i18n.t, "reference_not_saved:")).toBe(
      "The reference image could not be saved. Check the free disk space and write permissions.",
    );
  });

  it("keeps the backend's words when there is no known code", async () => {
    await i18n.changeLanguage("ja");
    expect(apiErrorText(i18n.t, new AgentApiError("background work failed: panic", 500))).toBe(
      "エラーが発生しました：background work failed: panic",
    );
    expect(apiErrorText(i18n.t, new AgentApiError("unknown_action", 400))).toBe("エラーが発生しました：unknown_action");
    // Keys that are not codes are never looked up from a detail.
    expect(apiErrorText(i18n.t, new AgentApiError("load", 500))).toBe("エラーが発生しました：load");
    expect(errorDetail(i18n.t, new Error("Failed to fetch"))).toBe("Failed to fetch");
    expect(errorDetail(i18n.t, "probe_unsupported")).toBe(
      "接続テストは、経路に「OpenAI 互換 API」を選んで保存したときだけ使えます。",
    );
  });
});
