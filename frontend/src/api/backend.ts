/**
 * Where the API lives and how to authenticate to it.
 *
 * In the Maskette window, the Rust side injects `window.__MASKETTE_BACKEND__` before the page loads: the local
 * API server's base URL (a random port on 127.0.0.1) and a token made fresh on every launch. `fetch` calls send
 * the token as a Bearer header; URLs the browser loads by itself (`<img>`, `EventSource`, download links) cannot,
 * so they carry it as a query parameter.
 *
 * Without the injected info (a plain browser on the Vite dev server, or tests), paths stay relative to `/api` and
 * the dev server's proxy adds the token.
 */

export type BackendInfo = { baseUrl: string; token: string };

declare global {
  interface Window {
    __MASKETTE_BACKEND__?: BackendInfo | undefined;
  }
}

/** Must match `QUERY_PARAM` in `src-tauri/src/backend/token.rs`. */
export const TOKEN_QUERY_PARAM = "maskette_token";

export function backendInfo(): BackendInfo | undefined {
  return typeof window === "undefined" ? undefined : window.__MASKETTE_BACKEND__;
}

function isApiPath(value: string): boolean {
  return value === "/api" || value.startsWith("/api/") || value.startsWith("/api?");
}

/** `fetch` an `/api/...` path on the backend, with the token attached. */
export function apiFetch(path: string, init?: RequestInit): Promise<Response> {
  const info = backendInfo();
  if (!info) return fetch(path, init);
  const headers = new Headers(init?.headers);
  headers.set("Authorization", `Bearer ${info.token}`);
  return fetch(new URL(path, info.baseUrl).toString(), { ...init, headers });
}

/**
 * A URL the browser can load on its own (`<img src>`, `EventSource`, `<a href>`). Turns an `/api/...` path into an
 * absolute backend URL carrying the token; anything else (blob:, data:, other sites) passes through untouched.
 */
export function apiUrl(path: string): string {
  const info = backendInfo();
  if (!info || !isApiPath(path)) return path;
  const url = new URL(path, info.baseUrl);
  url.searchParams.set(TOKEN_QUERY_PARAM, info.token);
  return url.toString();
}

/**
 * Rewrite every `/api/...` string in an API response through `apiUrl`. The backend hands out image and file links
 * as `/api/...` paths, which would otherwise resolve against the page's own origin instead of the backend.
 */
export function resolveApiUrls<T>(value: T): T {
  if (!backendInfo()) return value;
  return rewrite(value) as T;
}

function rewrite(value: unknown): unknown {
  if (typeof value === "string") return isApiPath(value) ? apiUrl(value) : value;
  if (Array.isArray(value)) return value.map(rewrite);
  if (value !== null && typeof value === "object" && Object.getPrototypeOf(value) === Object.prototype) {
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, rewrite(item)]));
  }
  return value;
}
