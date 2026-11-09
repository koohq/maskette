import type { TFunction } from "i18next";

/**
 * Failures the backend sends as a code with the values to fill in (`src-tauri/src/backend/failure.rs`):
 * the CLIs, the hosted APIs (sorted by HTTP status), missing settings, a stop, a refused reference.
 * Other failures come only as English text, which is shown as it is.
 */
export const FAILURE_CODES = new Set([
  "cli_not_found",
  "cli_start_failed",
  "cli_timeout",
  "cli_failed",
  "cli_failed_silently",
  "cli_login_required",
  "cli_usage_limit",
  "cli_offline",
  "cli_empty_reply",
  "cli_ran_command",
  "codex_home_unknown",
  "codex_no_image",
  "codex_bridge_removed",
  "api_unreachable",
  "api_unauthorized",
  "api_rate_limited",
  "api_server_error",
  "api_http_error",
  "api_timeout",
  "api_no_image",
  "api_key_missing",
  "setting_missing",
  "analysis_invalid",
  "cancelled",
  "reference_rejected",
  "no_references",
  "no_front_reference",
]);

/** Why the safety check refused a reference (`params.reason` of `reference_rejected`). */
const REJECT_REASONS = new Set(["reference_adult_explicit", "reference_unusable"]);

/**
 * A failure in the user's language when the backend gave a known code, else `fallback` (its English text).
 * The CLI's or API's own words (`params.detail`, often English) are added as they are.
 */
export function failureText(t: TFunction, code: unknown, params: unknown, fallback: string): string {
  if (typeof code !== "string" || !FAILURE_CODES.has(code)) return fallback;
  const values: Record<string, unknown> =
    params !== null && typeof params === "object" ? { ...(params as Record<string, unknown>) } : {};
  const reason = values["reason"];
  if (code === "reference_rejected") {
    values["reason"] =
      typeof reason === "string" && REJECT_REASONS.has(reason)
        ? t(`agent.failures.rejectReason.${reason}`)
        : t("agent.failures.rejectReason.reference_unusable");
  }
  const parts = [t(`agent.failures.${code}`, values)];
  const exitCode = values["exit_code"];
  if (typeof exitCode === "number") parts.push(t("agent.failures.exitCode", { exitCode }));
  if (values["outdated"] === true) parts.push(t("agent.failures.outdated"));
  const detail = values["detail"];
  if (typeof detail === "string" && detail.trim()) parts.push(t("agent.failures.detail", { detail }));
  return parts.join(t("agent.failures.gap"));
}
