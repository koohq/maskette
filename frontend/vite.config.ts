/// <reference types="vitest/config" />

import { readFileSync } from "node:fs";
import { request as httpRequest } from "node:http";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig, type Plugin } from "vite";

const buildDate = process.env.VITE_BUILD_DATE ?? new Date().toISOString();
const crossOriginIsolationHeaders = {
  "Cross-Origin-Embedder-Policy": "require-corp",
  "Cross-Origin-Opener-Policy": "same-origin",
} as const;
const immutableCacheHeader = "public, max-age=31536000, immutable";

function shouldLongCache(pathname = "") {
  return pathname.startsWith("/assets/") || pathname.startsWith("/models/") || pathname.startsWith("/mediapipe-wasm/");
}

function staticCacheHeadersPlugin(): Plugin {
  const middleware = (
    request: { url?: string },
    response: { setHeader: (name: string, value: string) => void },
    next: () => void,
  ) => {
    if (shouldLongCache(request.url?.split("?")[0] ?? "")) {
      response.setHeader("Cache-Control", immutableCacheHeader);
    }
    next();
  };

  return {
    name: "kig-static-cache-headers",
    configurePreviewServer(server) {
      server.middlewares.use(middleware);
    },
    configureServer(server) {
      server.middlewares.use(middleware);
    },
  };
}

// Written by the running app in debug builds (src-tauri/src/backend/dev_handoff.rs).
const devBackendFile = fileURLToPath(new URL("../.maskette-dev/backend.json", import.meta.url));

function readDevBackend(): { port: number; token: string } | null {
  try {
    const value = JSON.parse(readFileSync(devBackendFile, "utf8")) as { port?: unknown; token?: unknown };
    return typeof value.port === "number" && typeof value.token === "string"
      ? { port: value.port, token: value.token }
      : null;
  } catch {
    return null;
  }
}

function sendJson(response: import("node:http").ServerResponse, status: number, detail: string) {
  if (!response.headersSent) response.writeHead(status, { "Content-Type": "application/json" });
  response.end(JSON.stringify({ detail }));
}

/**
 * `/api` proxy for a plain browser on the dev server. The Maskette window talks to the backend directly with an
 * injected token; a browser tab has no token, so this forwards `/api/...` to the running app's backend (random port)
 * and adds it. Requests are streamed both ways, so uploads and server-sent events pass through.
 */
function devApiProxyPlugin(): Plugin {
  return {
    name: "maskette-dev-api-proxy",
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const url = request.url ?? "";
        if (url !== "/api" && !url.startsWith("/api/") && !url.startsWith("/api?")) {
          next();
          return;
        }
        const backend = readDevBackend();
        if (!backend) {
          sendJson(response, 503, "The Maskette backend is not running. Start the app with `pnpm dev`.");
          return;
        }
        const upstream = httpRequest(
          {
            headers: {
              ...request.headers,
              authorization: `Bearer ${backend.token}`,
              host: `127.0.0.1:${backend.port}`,
            },
            hostname: "127.0.0.1",
            method: request.method,
            path: url,
            port: backend.port,
          },
          (answer) => {
            response.writeHead(answer.statusCode ?? 502, answer.headers);
            answer.pipe(response);
          },
        );
        upstream.on("error", () => sendJson(response, 502, "The Maskette backend did not answer."));
        request.pipe(upstream);
      });
    },
  };
}

export default defineConfig({
  define: {
    __MASKETTE_BUILD_DATE__: JSON.stringify(buildDate),
  },
  plugins: [react(), staticCacheHeadersPlugin(), devApiProxyPlugin()],
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
  },
  server: {
    // Local only: the /api proxy adds the backend token, so the dev server must not be reachable from the LAN.
    host: "127.0.0.1",
    port: 5173,
    strictPort: true,
    headers: crossOriginIsolationHeaders,
  },
  preview: {
    headers: crossOriginIsolationHeaders,
  },
});
