import { z } from "zod";
import { apiFetch, resolveApiUrls } from "../../api/backend";

export const backendSettingsSchema = z.object({
  values: z.record(z.string(), z.string()),
  // API keys are write-only: only whether they are set, where from, and the last characters.
  secrets: z.record(z.string(), z.object({ set: z.boolean(), source: z.string(), hint: z.string() })),
  overridden: z.array(z.string()),
  options: z.record(z.string(), z.array(z.string())),
  status: z.record(z.string(), z.boolean()),
});

export type BackendSettings = z.infer<typeof backendSettingsSchema>;

const commonRequirements: Record<string, string> = {
  claude_code: "claude_cli",
  codex: "codex_cli",
  siliconflow: "siliconflow_key",
  ark: "ark_key",
  openrouter: "openrouter_key",
};

/** Status flag a backend needs to be usable; backends without an entry (fixtures) are always usable. */
export function backendRequirement(settingKey: string, option: string): string | undefined {
  if (option === "openai_compatible") return settingKey === "llm_provider" ? "analysis_llm_key" : "agent_llm_key";
  return commonRequirements[option];
}

/** Backends that drive a CLI signed in with a personal ChatGPT or Claude subscription. */
const subscriptionCliBackends = new Set(["codex", "claude_code"]);

export function usesSubscriptionCli(option: string): boolean {
  return subscriptionCliBackends.has(option);
}

/** The API-key backend each role recommends: it can serve other people, unlike a personal subscription. */
const recommendedBackends: Record<string, string> = {
  agent_llm_provider: "openai_compatible",
  llm_provider: "openai_compatible",
  image_provider: "openrouter",
};

export function isRecommendedBackend(settingKey: string, option: string): boolean {
  return recommendedBackends[settingKey] === option;
}

export const probeResultSchema = z.object({
  ok: z.boolean(),
  model: z.string(),
  base_url: z.string(),
  latency_ms: z.number().nullable().default(null),
  checks: z.array(z.object({ id: z.string(), status: z.enum(["ok", "warn", "fail"]), detail: z.string().default("") })),
});

export type ProbeResult = z.infer<typeof probeResultSchema>;
export type ProbeRole = "agent" | "analysis";

async function send(path: string, init?: RequestInit): Promise<unknown> {
  const response = await apiFetch(`/api/settings${path}`, init);
  if (!response.ok) {
    let detail = response.statusText;
    try {
      const body = (await response.json()) as { detail?: unknown };
      if (typeof body.detail === "string") detail = body.detail;
    } catch {
      // keep the status text
    }
    throw new Error(detail || `HTTP ${response.status}`);
  }
  return resolveApiUrls(await response.json());
}

async function request(init?: RequestInit): Promise<BackendSettings> {
  return backendSettingsSchema.parse(await send("", init));
}

/** Sends one small test request with the saved settings (costs a few tokens). */
export async function probeBackend(role: ProbeRole): Promise<ProbeResult> {
  return probeResultSchema.parse(await send(`/probe/${role}`, { method: "POST" }));
}

export function getBackendSettings() {
  return request();
}

/** A null value drops the saved change, so the default applies again. */
export function updateBackendSettings(values: Record<string, string | null>) {
  return request({ method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ values }) });
}

export function resetBackendSettings() {
  return request({ method: "DELETE" });
}
