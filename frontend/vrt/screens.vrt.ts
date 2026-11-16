import { expect, type Page, test } from "@playwright/test";
import en from "../src/i18n/resources/en";
import ja from "../src/i18n/resources/ja";
import zhCN from "../src/i18n/resources/zh-CN";
import { EMPTY_PROJECT_ID, mockBackend, PROJECT_ID } from "./backend";
import { compareWithKnown } from "./known-overflow";
import { findOverflow } from "./overflow";

const TEXTS = { ja, en, "zh-CN": zhCN } as const;

// Keys the app keeps in localStorage (i18n/locales.ts, app/theme.ts, features/workspace/autoFixPreferences.ts).
const LOCALE_KEY = "maskette.locale.v1";
const THEME_KEY = "maskette.theme.v1";
const AUTO_EYE_SYMMETRY_KEY = "maskette.autoEyeSymmetry.v1";

const LOCALES = ["ja", "en", "zh-CN"] as const;

// The Maskette window opens at 1280×720 and cannot be made smaller than 1024×640 (src-tauri/src/lib.rs).
const VIEWPORTS = [
  { name: "default", width: 1280, height: 720 },
  { name: "min", width: 1024, height: 640 },
] as const;

type Screen = {
  name: string;
  path: string;
  /** Shown once the screen has its data. */
  ready: (page: Page, locale: Locale) => Promise<void>;
};

type Locale = (typeof LOCALES)[number];

const SCREENS: Screen[] = [
  {
    name: "projects",
    path: "/",
    ready: (page) => expect(page.getByRole("img").first()).toBeVisible(),
  },
  {
    name: "workspace",
    path: `/p/${PROJECT_ID}`,
    // The editor finds the face landmarks when a version opens; that keeps the page busy for a while.
    ready: async (page, locale) => {
      await expect(page.locator(".ide-canvas-info")).toContainText(TEXTS[locale].workspace.status.landmarksReady, {
        timeout: 90_000,
      });
      // A tip about dragging landmarks shows for a few seconds after they are found.
      await expect(page.getByTestId("landmark-correction-hint")).toBeHidden({ timeout: 15_000 });
    },
  },
  {
    name: "workspace-empty",
    path: `/p/${EMPTY_PROJECT_ID}`,
    ready: (page) => expect(page.locator("textarea").first()).toBeVisible(),
  },
  {
    name: "settings",
    path: "/settings",
    ready: (page) => expect(page.locator("input").first()).toBeVisible(),
  },
];

async function open(page: Page, screen: Screen, locale: Locale) {
  const unanswered = await mockBackend(page);
  await page.clock.setFixedTime(new Date("2026-11-16T12:00:00+09:00"));
  await page.addInitScript(
    ([localeKey, themeKey, symmetryKey, value]) => {
      localStorage.setItem(localeKey, value);
      localStorage.setItem(themeKey, "dark");
      // A generated version would otherwise be corrected and saved as a new version as soon as it opens.
      localStorage.setItem(symmetryKey, "0");
    },
    [LOCALE_KEY, THEME_KEY, AUTO_EYE_SYMMETRY_KEY, locale] as const,
  );
  await page.goto(screen.path);
  await screen.ready(page, locale);
  await page.evaluate(() => document.fonts.ready);
  await expect.poll(() => page.evaluate(() => [...document.images].every((image) => image.complete))).toBe(true);
  expect(unanswered, "requests the fixtures do not answer").toEqual([]);
}

for (const screen of SCREENS) {
  for (const locale of LOCALES) {
    for (const viewport of VIEWPORTS) {
      const name = `${screen.name}-${locale}-${viewport.name}`;
      test(name, async ({ page }) => {
        await page.setViewportSize({ width: viewport.width, height: viewport.height });
        await open(page, screen, locale);

        const overflow = await findOverflow(page);
        await test.info().attach("overflow.json", {
          body: JSON.stringify(overflow, null, 2),
          contentType: "application/json",
        });
        // "…" is a deliberate cut (a long project name on a card); everything else is a layout bug.
        const broken = overflow.filter((finding) => finding.kind !== "truncated");
        const { unexpected, gone } = compareWithKnown(name, broken);
        expect.soft(unexpected, "text or controls that do not fit in their boxes").toEqual([]);
        expect.soft(gone, "known overflow this screen no longer shows (update vrt/known-overflow.ts)").toEqual([]);

        // The editor canvas is drawn by WebGL, which differs between GPUs; the screens are compared for layout.
        await expect(page).toHaveScreenshot(`${name}.png`, { mask: [page.locator("canvas")] });
      });
    }
  }
}
