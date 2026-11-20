import { z } from "zod";
import { apiFetch } from "../../api/backend";

/**
 * Updates to Maskette itself (`src-tauri/src/backend/update.rs`). Only release builds in the window support them;
 * the dev backend answers `supported: false`, and a plain browser never asks.
 */

const installSchema = z.discriminatedUnion("phase", [
  z.object({ phase: z.literal("idle") }),
  z.object({ phase: z.literal("downloading"), downloaded: z.number(), total: z.number().nullable() }),
  z.object({ phase: z.literal("installing") }),
  z.object({ phase: z.literal("failed"), detail: z.string() }),
]);

const statusSchema = z.object({
  supported: z.boolean(),
  current_version: z.string(),
  install: installSchema,
});

const releaseSchema = z.object({
  version: z.string(),
  notes: z.string().nullable(),
  /** Seconds since 1970. */
  published_at: z.number().nullable(),
});

const checkSchema = z.object({
  supported: z.boolean(),
  current_version: z.string(),
  update: releaseSchema.nullable(),
});

export type InstallState = z.output<typeof installSchema>;
export type UpdateStatus = z.output<typeof statusSchema>;
export type Release = z.output<typeof releaseSchema>;
export type CheckResult = z.output<typeof checkSchema>;

async function request<S extends z.ZodTypeAny>(path: string, schema: S, init?: RequestInit): Promise<z.output<S>> {
  const response = await apiFetch(path, init);
  if (!response.ok) {
    let detail = response.statusText || `HTTP ${response.status}`;
    try {
      const body = (await response.json()) as { detail?: unknown };
      if (typeof body.detail === "string") detail = body.detail;
    } catch {
      // keep the status text
    }
    throw new Error(detail);
  }
  return schema.parse(await response.json());
}

/** Whether updates are supported, the running version and how an install is going. No network. */
export function getUpdateStatus(): Promise<UpdateStatus> {
  return request("/api/update", statusSchema);
}

/** Asks the release server for a newer version. */
export function checkForUpdate(): Promise<CheckResult> {
  return request("/api/update/check", checkSchema, { method: "POST" });
}

/** Downloads `version` (the last check must have found it) and runs its installer, which closes the app. */
export function installUpdate(version: string): Promise<UpdateStatus> {
  return request("/api/update/install", statusSchema, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ version }),
  });
}
