import type { TFunction } from "i18next";

type Section = "agentSection" | "analysisSection" | "imageSection";
type Field = "base_url" | "model" | "api_key" | "extra_body" | "provider";

/**
 * Where each setting sits on the settings page, so an error can name it the way the page does instead of by its key.
 * The keys are the editable settings in `src-tauri/src/backend/settings/store.rs` (`settingLabels.test.ts` checks that
 * none is missing). The image providers' settings all sit under the image section, which shows only the chosen one.
 */
const SETTING_PLACES: Record<string, readonly [Section, Field]> = {
  agent_llm_provider: ["agentSection", "provider"],
  agent_llm_base_url: ["agentSection", "base_url"],
  agent_llm_model: ["agentSection", "model"],
  agent_llm_extra_body: ["agentSection", "extra_body"],
  agent_llm_api_key: ["agentSection", "api_key"],
  agent_codex_model: ["agentSection", "model"],
  agent_claude_code_model: ["agentSection", "model"],
  llm_provider: ["analysisSection", "provider"],
  analysis_llm_base_url: ["analysisSection", "base_url"],
  analysis_llm_model: ["analysisSection", "model"],
  analysis_llm_extra_body: ["analysisSection", "extra_body"],
  analysis_llm_api_key: ["analysisSection", "api_key"],
  image_provider: ["imageSection", "provider"],
  openrouter_base_url: ["imageSection", "base_url"],
  openrouter_image_model: ["imageSection", "model"],
  openrouter_extra_body: ["imageSection", "extra_body"],
  openrouter_api_key: ["imageSection", "api_key"],
  ark_base_url: ["imageSection", "base_url"],
  ark_image_model: ["imageSection", "model"],
  ark_api_key: ["imageSection", "api_key"],
  siliconflow_base_url: ["imageSection", "base_url"],
  siliconflow_image_model: ["imageSection", "model"],
  siliconflow_api_key: ["imageSection", "api_key"],
};

export const SETTING_KEYS = Object.keys(SETTING_PLACES);

/** A setting as the settings page names it, e.g. "API key (Chat assistant LLM)"; an unknown key stays as it is. */
export function settingLabel(t: TFunction, key: string): string {
  const place = SETTING_PLACES[key];
  if (!place) return key;
  const [section, field] = place;
  return t("settings.fieldIn", { section: t(`settings.${section}`), field: t(`settings.fieldNames.${field}`) });
}

/** The settings the backend names as one comma-separated value (`params.settings`), each by its name on the page. */
export function settingLabels(t: TFunction, keys: string): string {
  return keys
    .split(",")
    .map((key) => key.trim())
    .filter(Boolean)
    .map((key) => settingLabel(t, key))
    .join(t("settings.fieldSeparator"));
}

/** What the backend checks a setting's value for (`valid_setting`), so a refusal can say what to fix. */
export function settingKind(key: string): "secret" | "extra_body" | "other" {
  if (key.endsWith("_api_key")) return "secret";
  if (key.endsWith("_extra_body")) return "extra_body";
  return "other";
}
