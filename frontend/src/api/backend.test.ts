import { afterEach, describe, expect, it, vi } from "vitest";
import { apiFetch, apiUrl, resolveApiUrls, TOKEN_QUERY_PARAM } from "./backend";

const info = { baseUrl: "http://127.0.0.1:4321", token: "test-token" };

afterEach(() => {
  delete window.__MASKETTE_BACKEND__;
  vi.unstubAllGlobals();
});

describe("without the injected backend (plain browser on the dev server)", () => {
  it("keeps paths relative so the dev proxy handles them", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response("{}"));
    vi.stubGlobal("fetch", fetchMock);

    await apiFetch("/api/health", { method: "GET" });

    expect(fetchMock).toHaveBeenCalledWith("/api/health", { method: "GET" });
    expect(apiUrl("/api/generated/a.png")).toBe("/api/generated/a.png");
    const body = { url: "/api/generated/a.png" };
    expect(resolveApiUrls(body)).toBe(body);
  });
});

describe("with the injected backend (Maskette window)", () => {
  it("sends fetches to the backend with the bearer token", async () => {
    window.__MASKETTE_BACKEND__ = info;
    const fetchMock = vi.fn().mockResolvedValue(new Response("{}"));
    vi.stubGlobal("fetch", fetchMock);

    await apiFetch("/api/settings", { headers: { "Content-Type": "application/json" }, method: "PUT" });

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("http://127.0.0.1:4321/api/settings");
    expect(init.method).toBe("PUT");
    const headers = new Headers(init.headers);
    expect(headers.get("Authorization")).toBe("Bearer test-token");
    expect(headers.get("Content-Type")).toBe("application/json");
  });

  it("puts the token into URLs the browser loads by itself", () => {
    window.__MASKETTE_BACKEND__ = info;

    expect(apiUrl("/api/agent/conversations/c1/events?after=3")).toBe(
      `http://127.0.0.1:4321/api/agent/conversations/c1/events?after=3&${TOKEN_QUERY_PARAM}=test-token`,
    );
    expect(apiUrl("blob:http://127.0.0.1:5173/abc")).toBe("blob:http://127.0.0.1:5173/abc");
    expect(apiUrl("https://example.com/api/x")).toBe("https://example.com/api/x");
  });

  it("rewrites /api links anywhere in a response", () => {
    window.__MASKETTE_BACKEND__ = info;

    const resolved = resolveApiUrls({
      count: 2,
      items: [{ image_url: "/api/generated/a.png", note: "keeps /api/ in prose" }, { image_url: null }],
      title: "/apiary",
    });

    expect(resolved).toEqual({
      count: 2,
      items: [
        { image_url: `http://127.0.0.1:4321/api/generated/a.png?${TOKEN_QUERY_PARAM}=test-token`, note: "keeps /api/ in prose" },
        { image_url: null },
      ],
      title: "/apiary",
    });
  });
});
