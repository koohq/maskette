// After a release is published, GitHub can take a while to point releases/latest at it, and until then installed
// copies keep finding the previous latest.json. These helpers ask the updater's endpoint until it names the new
// version, so the release script can say when the update is really there.

/** The version a latest.json at `endpoint` names, or a short reason it could not be read. */
export type LatestAnswer = { version: string } | { problem: string };

export async function fetchLatestVersion(endpoint: string, fetchImpl: typeof fetch = fetch): Promise<LatestAnswer> {
  let response: Response;
  try {
    // Ask past any cache on the way; the point is to see what an installed copy would get now.
    response = await fetchImpl(endpoint, { headers: { "Cache-Control": "no-cache" } });
  } catch (error) {
    return { problem: error instanceof Error ? error.message : String(error) };
  }
  if (!response.ok) return { problem: `HTTP ${response.status}` };
  try {
    const body: unknown = await response.json();
    const version = typeof body === "object" && body !== null ? (body as { version?: unknown }).version : undefined;
    return typeof version === "string" && version ? { version } : { problem: "no version in latest.json" };
  } catch {
    return { problem: "latest.json is not JSON" };
  }
}

export interface WaitOptions {
  endpoint: string;
  /** The version just published. */
  version: string;
  timeoutMs: number;
  intervalMs: number;
  fetchLatest?: (endpoint: string) => Promise<LatestAnswer>;
  sleep?: (ms: number) => Promise<void>;
  now?: () => number;
  /** Called with each answer that is not yet the new version, when it differs from the one before. */
  onWaiting?: (answer: LatestAnswer, elapsedMs: number) => void;
}

export type WaitResult = { seen: true; elapsedMs: number } | { seen: false; elapsedMs: number; last: LatestAnswer };

/** Asks `endpoint` every `intervalMs` until it names `version` or `timeoutMs` has passed. */
export async function waitForLatest(options: WaitOptions): Promise<WaitResult> {
  const fetchLatest = options.fetchLatest ?? ((endpoint: string) => fetchLatestVersion(endpoint));
  const sleep = options.sleep ?? ((ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms)));
  const now = options.now ?? Date.now;
  const startedAt = now();
  let previous = "";
  for (;;) {
    const answer = await fetchLatest(options.endpoint);
    const elapsedMs = now() - startedAt;
    if ("version" in answer && answer.version === options.version) return { seen: true, elapsedMs };
    const said = describeAnswer(answer);
    if (said !== previous) {
      previous = said;
      options.onWaiting?.(answer, elapsedMs);
    }
    if (elapsedMs + options.intervalMs > options.timeoutMs) return { seen: false, elapsedMs, last: answer };
    await sleep(options.intervalMs);
  }
}

export function describeAnswer(answer: LatestAnswer): string {
  return "version" in answer
    ? `latest.json says ${answer.version}`
    : `latest.json could not be read (${answer.problem})`;
}

/** "45s", "2m 05s". */
export function formatDuration(ms: number): string {
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}m ${String(seconds % 60).padStart(2, "0")}s`;
}
