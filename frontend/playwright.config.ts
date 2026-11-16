import { defineConfig } from "@playwright/test";

// Visual regression tests: the screens are rendered by the Vite dev server with every `/api` request answered by
// fixtures (vrt/backend.ts), so no Maskette backend has to run. Baselines are per platform, because fonts differ.
const PORT = 5174;

export default defineConfig({
  testDir: "./vrt",
  testMatch: "**/*.vrt.ts",
  snapshotPathTemplate: "{testDir}/__screenshots__/{testFileName}/{arg}-{platform}{ext}",
  outputDir: "./vrt-results",
  reporter: [["list"], ["html", { open: "never", outputFolder: "./vrt-report" }]],
  // The workspace screen finds face landmarks on the CPU before it settles. Several at once starve each other until
  // they time out, while one at a time takes a few seconds each.
  workers: 1,
  timeout: 120_000,
  expect: {
    toHaveScreenshot: { animations: "disabled", caret: "hide", maxDiffPixelRatio: 0.001 },
  },
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    browserName: "chromium",
    deviceScaleFactor: 1,
    timezoneId: "Asia/Tokyo",
  },
  webServer: {
    command: `pnpm exec vite --port ${PORT} --strictPort`,
    url: `http://127.0.0.1:${PORT}`,
    reuseExistingServer: true,
  },
});
