/**
 * Takes the README screenshots (`.github/readme/`) from the real app and its sample project, one language at a time:
 * starts the debug app (`pnpm dev`) on an empty data folder with the sample in that language, drives its window over
 * the DevTools protocol and saves each picture as WebP.
 *
 * Windows only (the window is WebView2). Run it from PowerShell, where cargo finds the MSVC linker:
 * `pnpm screenshots [en|ja|zh-CN ...]`. With `--attach`, it uses a window that is already open (started by hand with
 * `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333`) instead, for the language it is in.
 */
import { type ChildProcess, execFileSync, spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { type Browser, type CDPSession, chromium, type Locator, type Page } from "@playwright/test";

const REPO = fileURLToPath(new URL("../..", import.meta.url));
const OUT = join(REPO, ".github", "readme");
const PORT = 9333;
const LOCALES = ["en", "ja", "zh-CN"] as const;
type Locale = (typeof LOCALES)[number];

// The window's size (src-tauri/src/lib.rs), drawn at 1.5x so the pictures stay sharp on high-DPI screens.
const VIEWPORT = { width: 1280, height: 720, deviceScaleFactor: 1.5, mobile: false };
const QUALITY = 90;

// Keys the app keeps in localStorage (i18n/locales.ts, app/theme.ts, features/workspace/autoFixPreferences.ts).
const PREFERENCES = (locale: Locale) => ({
  "maskette.locale.v1": locale,
  "maskette.theme.v1": "light",
  // Opening a generated version would otherwise correct its eyes and save a new version.
  "maskette.autoEyeSymmetry.v1": "0",
});

// The sample's versions (src-tauri/sample/project.json), in the order the chat shows them.
const VERSIONS = ["front-1", "turnaround-1", "eyes-1"] as const;
type Version = (typeof VERSIONS)[number];

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function waitFor<T>(what: string, probe: () => Promise<T | undefined>, timeoutMs: number): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const value = await probe().catch(() => undefined);
    if (value !== undefined) return value;
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await sleep(500);
  }
}

/** Starts the debug app on a new data folder; the sample project comes in `locale`. */
function startApp(locale: Locale, folder: string): ChildProcess {
  const child = spawn("pnpm", ["dev"], {
    cwd: REPO,
    shell: true,
    stdio: ["ignore", "inherit", "inherit"],
    env: {
      ...process.env,
      MASKETTE_DATA_DIR: join(folder, "data"),
      MASKETTE_SAMPLE_LOCALE: locale,
      WEBVIEW2_USER_DATA_FOLDER: join(folder, "webview"),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}`,
    },
  });
  return child;
}

function stopApp(child: ChildProcess) {
  if (child.pid === undefined) return;
  // pnpm → tauri → the app and Vite: end the whole tree.
  try {
    execFileSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" });
  } catch {
    // Already gone.
  }
}

async function connect(): Promise<{ browser: Browser; page: Page }> {
  const browser = await waitFor(
    "the app's window",
    async () => {
      const response = await fetch(`http://127.0.0.1:${PORT}/json/version`);
      return response.ok ? chromium.connectOverCDP(`http://127.0.0.1:${PORT}`) : undefined;
    },
    15 * 60_000,
  );
  const page = await waitFor(
    "the app's page",
    async () =>
      browser
        .contexts()[0]
        ?.pages()
        .find((candidate) => candidate.url().startsWith("http")),
    120_000,
  );
  return { browser, page };
}

async function capture(cdp: CDPSession, name: string) {
  const { data } = await cdp.send("Page.captureScreenshot", { format: "webp", quality: QUALITY });
  const file = join(OUT, `${name}.webp`);
  writeFileSync(file, Buffer.from(data, "base64"));
  console.log(`saved ${file}`);
}

/** Opens a version from the list on the left and waits until the editor has looked for its landmarks. */
async function openVersion(page: Page, version: Version) {
  await page.locator(".ide-tree-item-id").getByText(version, { exact: true }).click();
  // Only this version's tab stays open.
  const others = page.locator(".ide-tab[data-active='false'] .ide-tab-close");
  while ((await others.count()) > 0) await others.first().click();
  const info = page.locator(".ide-canvas-info");
  // The status line (CanvasInfo in WorkspacePage.tsx) names the version and, fifth, whether its landmarks were found,
  // with "…" while the editor is still looking. Settled once it reads the same for a while.
  let last = "";
  let since = Date.now();
  await waitFor(
    `${version} with its landmarks`,
    async () => {
      const text = (await info.textContent()) ?? "";
      if (text !== last) {
        last = text;
        since = Date.now();
      }
      const done = text.startsWith(`${version} · `) && text.split(" · ").length >= 5 && !text.includes("…");
      return done && Date.now() - since > 1500 ? true : undefined;
    },
    120_000,
  );
  // A tip about dragging landmarks shows for a few seconds after they are found.
  await page
    .getByTestId("landmark-correction-hint")
    .waitFor({ state: "hidden", timeout: 15_000 })
    .catch(() => {});
}

/**
 * Turns off the landmark marks the editor draws over a face it has found (the eye button in its toolbar), so the
 * picture shows the face itself.
 */
async function hideLandmarks(page: Page) {
  const stage = page.getByTestId("landmark-stage");
  if ((await stage.count()) === 0) return;
  await page.getByTestId("landmark-visibility-toggle").click();
  await stage.waitFor({ state: "detached", timeout: 10_000 });
}

/** Scrolls the chat so `card` sits at `block` of the visible part. */
async function showInChat(card: Locator, block: "start" | "center" | "end") {
  await card.evaluate((element, where) => element.scrollIntoView({ block: where }), block);
  await card.page().waitForTimeout(400);
}

async function settle(page: Page) {
  await page.evaluate(() => document.fonts.ready);
  await page.waitForFunction(() => [...document.images].every((image) => image.complete));
  await page.mouse.move(0, VIEWPORT.height - 1);
  await page.waitForTimeout(600);
}

async function shoot(page: Page, locale: Locale) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Emulation.setDeviceMetricsOverride", VIEWPORT);
  await page.evaluate((entries) => {
    for (const [key, value] of Object.entries(entries)) localStorage.setItem(key, value);
  }, PREFERENCES(locale));
  await page.goto(new URL("/", page.url()).href);
  // The sample is the only project.
  await page
    .locator("main button")
    .filter({ has: page.locator("img") })
    .first()
    .click();
  await page.locator(".ide-agent-scroll").waitFor();

  const card = (version: Version) => page.locator(".ide-agent-scroll .agent-image-card").nth(VERSIONS.indexOf(version));

  await openVersion(page, "front-1");
  await hideLandmarks(page);
  await showInChat(card("turnaround-1"), "end");
  await settle(page);
  await capture(cdp, `workspace-${locale}`);

  await openVersion(page, "eyes-1");
  await showInChat(card("eyes-1"), "end");
  await settle(page);
  await capture(cdp, `eye-closeup-${locale}`);

  await openVersion(page, "turnaround-1");
  await showInChat(page.locator(".ide-agent-scroll .agent-notes-card"), "start");
  await settle(page);
  await capture(cdp, `design-notes-${locale}`);

  await cdp.send("Emulation.clearDeviceMetricsOverride");
  await cdp.detach();
}

const args = process.argv.slice(2);
const attach = args.includes("--attach");
const wanted = args.filter((arg): arg is Locale => (LOCALES as readonly string[]).includes(arg));
mkdirSync(OUT, { recursive: true });

if (attach) {
  const { browser, page } = await connect();
  const locale = wanted[0] ?? "en";
  await shoot(page, locale);
  await browser.close();
} else {
  for (const locale of wanted.length > 0 ? wanted : LOCALES) {
    const folder = mkdtempSync(join(tmpdir(), "maskette-screenshots-"));
    const app = startApp(locale, folder);
    try {
      const { browser, page } = await connect();
      await shoot(page, locale);
      await browser.close();
    } finally {
      stopApp(app);
      // The app needs a moment to let go of its files.
      await sleep(2000);
      rmSync(folder, { recursive: true, force: true, maxRetries: 5, retryDelay: 1000 });
    }
  }
}
