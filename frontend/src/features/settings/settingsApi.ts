import { z } from "zod";
import { apiFetch, resolveApiUrls } from "../../api/backend";

export const backendSettingsSchema = z.object({
  values: z.record(z.string(), z.string()),
  // API keys are kept per service (`serviceOf`) and are write-only: only which services have one, and its last
  // characters.
  keys: z.record(z.string(), z.object({ hint: z.string() })),
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

/** The analysis choice that runs analysis on the chat assistant's backend, URL, key and model (`SAME_AS_AGENT` in `store.rs`). */
export const SAME_AS_AGENT = "same_as_agent";

/** The backend analysis runs on: the chat assistant's while it follows the chat (both name backends alike). */
export function analysisBackend(values: Record<string, string>): string | undefined {
  const choice = values["llm_provider"];
  return choice === SAME_AS_AGENT ? values["agent_llm_provider"] : choice;
}

/**
 * The two sites of a service that runs a mainland China one and an international one. They keep separate accounts and
 * API keys, and a key works only on the site that issued it.
 */
export const URL_REGIONS = ["mainland", "international"] as const;

export type UrlRegion = (typeof URL_REGIONS)[number];

export interface UrlPreset {
  id: "openrouter" | "openai" | "gemini" | "ark" | "siliconflow";
  /** The URL the preset fills in: the mainland China site's for a service with two sites. */
  url: string;
  /** The international site's URL, for a service with two sites. */
  international?: string;
}

/**
 * API URLs the chat assistant and analysis can start from. Volcengine Ark's is the default Ark URL in `store.rs` (also
 * the old VSK default for the chat assistant), so the chat and Ark image generation share a key; SiliconFlow's is its
 * default too. `settingsApi.test.ts` checks them against `store.rs`. Ark and SiliconFlow also run international sites
 * (BytePlus ModelArk for Ark), which this app has not been tried on.
 */
export const URL_PRESETS: readonly UrlPreset[] = [
  { id: "openrouter", url: "https://openrouter.ai/api/v1" },
  { id: "openai", url: "https://api.openai.com/v1" },
  { id: "gemini", url: "https://generativelanguage.googleapis.com/v1beta/openai" },
  {
    id: "ark",
    url: "https://ark.cn-beijing.volces.com/api/plan/v3",
    international: "https://ark.ap-southeast.bytepluses.com/api/v3",
  },
  { id: "siliconflow", url: "https://api.siliconflow.cn/v1", international: "https://api.siliconflow.com/v1" },
];

export function urlPreset(id: UrlPreset["id"]): UrlPreset {
  const preset = URL_PRESETS.find((candidate) => candidate.id === id);
  if (!preset) throw new Error(`No URL preset ${id}`);
  return preset;
}

/** The URL of a preset's site in `region`; a service with one site has the same URL in both. */
export function regionUrl(preset: UrlPreset, region: UrlRegion): string {
  return region === "international" && preset.international ? preset.international : preset.url;
}

export interface PresetMatch {
  preset: UrlPreset;
  /** Which site the URL is, for a service with two sites. */
  region?: UrlRegion | undefined;
}

export const normalizeUrl = (url = "") => url.trim().replace(/\/+$/, "");

/**
 * The service an API URL belongs to, which its key is saved under: the URL's host, with the port unless it is the
 * scheme's default. A key comes from an account at one site, so every OpenRouter URL shares one, while the two sites of
 * Ark or SiliconFlow keep one each. A URL that does not parse is a service of its own, and an empty one none. The same
 * rule as `service_of` in `store.rs`.
 */
export function serviceOf(url: string | undefined): string {
  const normalized = normalizeUrl(url);
  try {
    const { host } = new URL(normalized);
    if (host) return host;
  } catch {
    // not a URL: a service of its own
  }
  return normalized;
}

/** The preset, and for a service with two sites the site, that a service (`serviceOf`) is. */
export function presetOfService(service: string): PresetMatch | undefined {
  for (const preset of URL_PRESETS) {
    if (serviceOf(preset.url) === service) return preset.international ? { preset, region: "mainland" } : { preset };
    if (preset.international && serviceOf(preset.international) === service) return { preset, region: "international" };
  }
  return undefined;
}

/** The preset, and for a service with two sites the site, whose URL `url` is. */
export function matchingPreset(
  url: string | undefined,
  presets: readonly UrlPreset[] = URL_PRESETS,
): PresetMatch | undefined {
  const normalized = normalizeUrl(url);
  for (const preset of presets) {
    if (preset.url === normalized) return preset.international ? { preset, region: "mainland" } : { preset };
    if (preset.international === normalized) return { preset, region: "international" };
  }
  return undefined;
}

const priceSchema = z.object({
  // `token`: US dollars per million tokens; `image`: per image.
  unit: z.enum(["token", "image"]),
  input: z.number().nullable().default(null),
  output: z.number().nullable().default(null),
});

export const modelCandidatesSchema = z.object({
  models: z.array(
    z.object({ id: z.string(), name: z.string().default(""), price: priceSchema.nullable().default(null) }),
  ),
});

export type ModelCandidate = z.infer<typeof modelCandidatesSchema>["models"][number];
export type ModelPrice = z.infer<typeof priceSchema>;
export type ModelRole = "agent" | "analysis" | "image";

/** The models a role's model field offers for `baseUrl`: OpenRouter's that suit the role, or none elsewhere. */
export async function getModelCandidates(role: ModelRole, baseUrl: string): Promise<ModelCandidate[]> {
  const query = new URLSearchParams({ base_url: baseUrl });
  return modelCandidatesSchema.parse(await send(`/models/${role}?${query}`)).models;
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

/**
 * Saves changed settings and API keys by service. A null setting drops the saved change, so the default applies again;
 * a null key removes the service's key.
 */
export function updateBackendSettings(values: Record<string, string | null>, keys: Record<string, string | null> = {}) {
  const body = Object.keys(keys).length > 0 ? { values, keys } : { values };
  return request({ method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
}

export function resetBackendSettings() {
  return request({ method: "DELETE" });
}
