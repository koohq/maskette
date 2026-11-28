import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { fetchLatestVersion, formatDuration, type LatestAnswer, waitForLatest } from "./wait-for-latest.ts";

const ENDPOINT = "https://example.test/releases/latest/download/latest.json";

/** A clock that only moves when the code under test sleeps. */
function fakeTime() {
  let now = 0;
  return {
    now: () => now,
    sleep: async (ms: number) => {
      now += ms;
    },
  };
}

/** Gives the answers in order, then repeats the last one. */
function answers(...list: LatestAnswer[]) {
  const asked: string[] = [];
  return {
    asked,
    fetchLatest: async (endpoint: string) => {
      asked.push(endpoint);
      return list[Math.min(asked.length, list.length) - 1] ?? { problem: "no answer" };
    },
  };
}

describe("waitForLatest", () => {
  it("returns at once when latest.json already names the new version", async () => {
    const time = fakeTime();
    const server = answers({ version: "0.1.2" });
    const result = await waitForLatest({
      endpoint: ENDPOINT,
      version: "0.1.2",
      timeoutMs: 60_000,
      intervalMs: 10_000,
      ...time,
      fetchLatest: server.fetchLatest,
    });
    assert.deepEqual(result, { seen: true, elapsedMs: 0 });
    assert.deepEqual(server.asked, [ENDPOINT]);
  });

  it("keeps asking until GitHub switches to the new version, and says what it saw once per change", async () => {
    const time = fakeTime();
    const server = answers(
      { version: "0.1.1" },
      { version: "0.1.1" },
      { problem: "HTTP 404" },
      { version: "0.1.1" },
      { version: "0.1.2" },
    );
    const seen: string[] = [];
    const result = await waitForLatest({
      endpoint: ENDPOINT,
      version: "0.1.2",
      timeoutMs: 600_000,
      intervalMs: 10_000,
      ...time,
      fetchLatest: server.fetchLatest,
      onWaiting: (answer, elapsedMs) =>
        seen.push(`${"version" in answer ? answer.version : answer.problem}@${elapsedMs}`),
    });
    assert.deepEqual(result, { seen: true, elapsedMs: 40_000 });
    assert.equal(server.asked.length, 5);
    assert.deepEqual(seen, ["0.1.1@0", "HTTP 404@20000", "0.1.1@30000"]);
  });

  it("gives up after the timeout and reports the last answer", async () => {
    const time = fakeTime();
    const server = answers({ version: "0.1.1" });
    const result = await waitForLatest({
      endpoint: ENDPOINT,
      version: "0.1.2",
      timeoutMs: 30_000,
      intervalMs: 10_000,
      ...time,
      fetchLatest: server.fetchLatest,
    });
    assert.deepEqual(result, { seen: false, elapsedMs: 30_000, last: { version: "0.1.1" } });
    assert.equal(server.asked.length, 4);
  });
});

describe("fetchLatestVersion", () => {
  const respond = (response: Response | Error): typeof fetch =>
    (async () => {
      if (response instanceof Error) throw response;
      return response;
    }) as typeof fetch;

  it("reads the version from latest.json", async () => {
    const answer = await fetchLatestVersion(ENDPOINT, respond(Response.json({ version: "0.1.2", platforms: {} })));
    assert.deepEqual(answer, { version: "0.1.2" });
  });

  it("asks past caches", async () => {
    let headers: Headers | undefined;
    await fetchLatestVersion(ENDPOINT, (async (_input: unknown, init?: RequestInit) => {
      headers = new Headers(init?.headers);
      return Response.json({ version: "0.1.2" });
    }) as typeof fetch);
    assert.equal(headers?.get("cache-control"), "no-cache");
  });

  it("turns an HTTP error, a body that is not latest.json and a network failure into a reason", async () => {
    assert.deepEqual(await fetchLatestVersion(ENDPOINT, respond(new Response("Not Found", { status: 404 }))), {
      problem: "HTTP 404",
    });
    assert.deepEqual(await fetchLatestVersion(ENDPOINT, respond(new Response("<html>"))), {
      problem: "latest.json is not JSON",
    });
    assert.deepEqual(await fetchLatestVersion(ENDPOINT, respond(Response.json({ notes: "x" }))), {
      problem: "no version in latest.json",
    });
    assert.deepEqual(await fetchLatestVersion(ENDPOINT, respond(new Error("getaddrinfo ENOTFOUND"))), {
      problem: "getaddrinfo ENOTFOUND",
    });
  });
});

describe("formatDuration", () => {
  it("writes seconds, and minutes with two-digit seconds", () => {
    assert.equal(formatDuration(0), "0s");
    assert.equal(formatDuration(44_600), "45s");
    assert.equal(formatDuration(125_000), "2m 05s");
  });
});
