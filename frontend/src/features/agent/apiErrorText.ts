import type { TFunction } from "i18next";

/**
 * Codes the backend sends as the `detail` of an HTTP error (`src-tauri/src/backend/error.rs` and the routers) that a
 * user can run into, translated as `agent.errors.<code>`. Failures inside a run (the CLIs, the hosted APIs) come as
 * events instead and are translated by `failureText.ts`; the two sets of codes do not overlap.
 *
 * `apiErrorCodes.test.ts` reads the backend's source and fails when it sends a code that is neither here nor in its
 * list of codes the screen never shows, so a new code cannot slip through untranslated.
 */
export const DETAIL_CODES = new Set([
  "conversation_not_found",
  "conversation_running",
  "message_not_found",
  "nothing_to_regenerate",
  "empty_message",
  "too_many_files",
  "too_many_references",
  "unsupported_image_type",
  "unknown_image",
  "image_too_large",
  "image_invalid",
  "edit_note_required",
  "recipe_too_large",
  "recipe_invalid",
  "not_a_vkp_file",
  "project_too_large",
  "not_a_zip",
  "missing_manifest",
  "invalid_json",
  "invalid_project",
  "unknown_format",
  "unsafe_entry",
  "import_not_saved",
  "invalid_setting",
  "probe_unsupported",
  "settings_not_saved",
  "version_not_saved",
  "reference_not_saved",
]);

/**
 * Codes that carry a value after a colon: `invalid_setting:<key>` names the setting; the `..._not_saved: <error>`
 * ones add the system's own words, which are shown as they are (like `detail` in `failureText.ts`).
 */
const VALUE_PARAMS: Record<string, "key" | "detail"> = {
  invalid_setting: "key",
  settings_not_saved: "detail",
  version_not_saved: "detail",
  reference_not_saved: "detail",
};

/** The `detail` of an HTTP error in the user's language when it is a known code, else `null`. */
export function detailText(t: TFunction, detail: string): string | null {
  const match = /^([a-z][a-z0-9_]*)(?::\s?([\s\S]*))?$/.exec(detail.trim());
  const code = match?.[1];
  if (!code || !DETAIL_CODES.has(code)) return null;
  const value = match[2]?.trim() ?? "";
  const param = VALUE_PARAMS[code];
  if (param === "key") return t(`agent.errors.${code}`, { key: value });
  const text = t(`agent.errors.${code}`);
  if (param === "detail" && value)
    return [text, t("agent.failures.detail", { detail: value })].join(t("agent.failures.gap"));
  return text;
}

/** Why an API call failed, for a message that already says what failed ("Saving failed: …"). */
export function errorDetail(t: TFunction, error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return detailText(t, message) ?? message;
}

/**
 * A failed API call for the screen: the translated code, or "Something went wrong: <text>" with the backend's or the
 * browser's own words when there is no code.
 */
export function apiErrorText(t: TFunction, error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return detailText(t, message) ?? t("agent.errors.generic", { message });
}
