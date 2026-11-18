import { expect, type Page, test } from "@playwright/test";
import type { EditorTool } from "../src/features/editor/components/EditorToolRail";
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
type Locale = (typeof LOCALES)[number];
type Texts = (typeof TEXTS)[Locale];
type Theme = "dark" | "light";

// The editor tools in the order of the activity bar (EditorToolRail.tsx).
const TOOLS = Object.keys(ja.workspace.tools) as EditorTool[];

// The Maskette window opens at 1280×720 and cannot be made smaller than 1024×640 (src-tauri/src/lib.rs).
const DEFAULT_SIZE = { name: "default", width: 1280, height: 720 } as const;
const MIN_SIZE = { name: "min", width: 1024, height: 640 } as const;
type Viewport = typeof DEFAULT_SIZE | typeof MIN_SIZE;

type Screen = {
  name: string;
  path: string;
  /** Shown once the screen has its data. */
  ready: (page: Page, text: Texts) => Promise<void>;
};

const PROJECTS: Screen = {
  name: "projects",
  path: "/",
  ready: (page) => expect(page.getByRole("img").first()).toBeVisible(),
};

const WORKSPACE: Screen = {
  name: "workspace",
  path: `/p/${PROJECT_ID}`,
  // The editor finds the face landmarks when a version opens; that keeps the page busy for a while.
  ready: async (page, text) => {
    await expect(page.locator(".ide-canvas-info")).toContainText(text.workspace.status.landmarksReady, {
      timeout: 90_000,
    });
    // A tip about dragging landmarks shows for a few seconds after they are found.
    await expect(page.getByTestId("landmark-correction-hint")).toBeHidden({ timeout: 15_000 });
  },
};

const WORKSPACE_EMPTY: Screen = {
  name: "workspace-empty",
  path: `/p/${EMPTY_PROJECT_ID}`,
  ready: (page) => expect(page.locator("textarea").first()).toBeVisible(),
};

const SETTINGS: Screen = {
  name: "settings",
  path: "/settings",
  ready: (page) => expect(page.locator("input").first()).toBeVisible(),
};

const SCREENS = [PROJECTS, WORKSPACE, WORKSPACE_EMPTY, SETTINGS];

/** Opens a screen with the fixtures; returns the requests the fixtures do not answer, filled in as the test goes. */
async function open(page: Page, screen: Screen, locale: Locale, viewport: Viewport, theme: Theme = "dark") {
  await page.setViewportSize({ width: viewport.width, height: viewport.height });
  const unanswered = await mockBackend(page);
  await page.clock.setFixedTime(new Date("2026-11-16T12:00:00+09:00"));
  await page.addInitScript(
    ([localeKey, themeKey, symmetryKey, localeValue, themeValue]) => {
      localStorage.setItem(localeKey, localeValue);
      localStorage.setItem(themeKey, themeValue);
      // A generated version would otherwise be corrected and saved as a new version as soon as it opens.
      localStorage.setItem(symmetryKey, "0");
    },
    [LOCALE_KEY, THEME_KEY, AUTO_EYE_SYMMETRY_KEY, locale, theme] as const,
  );
  await page.goto(screen.path);
  await screen.ready(page, TEXTS[locale]);
  return unanswered;
}

async function settle(page: Page) {
  await page.evaluate(() => document.fonts.ready);
  await expect.poll(() => page.evaluate(() => [...document.images].every((image) => image.complete))).toBe(true);
}

/** Checks the page for overflow, then compares it with the baseline `<name>.png`. Soft, so one test can check several. */
async function inspect(page: Page, name: string, unanswered: string[]) {
  await settle(page);
  expect.soft(unanswered, `${name}: requests the fixtures do not answer`).toEqual([]);

  const overflow = await findOverflow(page);
  await test.info().attach(`${name}-overflow.json`, {
    body: JSON.stringify(overflow, null, 2),
    contentType: "application/json",
  });
  // "…" is a deliberate cut (a long project name on a card); everything else is a layout bug.
  const broken = overflow.filter((finding) => finding.kind !== "truncated");
  const { unexpected, gone } = compareWithKnown(name, broken);
  expect.soft(unexpected, `${name}: text or controls that do not fit in their boxes`).toEqual([]);
  expect.soft(gone, `${name}: known overflow this screen no longer shows (update vrt/known-overflow.ts)`).toEqual([]);

  // The editor canvas is drawn by WebGL, which differs between GPUs; the screens are compared for layout.
  await expect.soft(page).toHaveScreenshot(`${name}.png`, { mask: [page.locator("canvas")] });
}

// Every screen in the dark theme, at both sizes.
for (const screen of SCREENS) {
  for (const locale of LOCALES) {
    for (const viewport of [DEFAULT_SIZE, MIN_SIZE]) {
      const name = `${screen.name}-${locale}-${viewport.name}`;
      test(name, async ({ page }) => {
        const unanswered = await open(page, screen, locale, viewport);
        await inspect(page, name, unanswered);
      });
    }
  }
}

// The light theme, at the smaller size, where things are tightest.
for (const screen of SCREENS) {
  for (const locale of LOCALES) {
    const name = `${screen.name}-light-${locale}-${MIN_SIZE.name}`;
    test(name, async ({ page }) => {
      const unanswered = await open(page, screen, locale, MIN_SIZE, "light");
      await inspect(page, name, unanswered);
    });
  }
}

// Each editor tool's panel in the left side bar, with its buttons above the canvas. One test per language, so the
// landmarks are found once for all the tools.
for (const locale of LOCALES) {
  test(`workspace-tools-${locale}-${MIN_SIZE.name}`, async ({ page }) => {
    const text = TEXTS[locale];
    const unanswered = await open(page, WORKSPACE, locale, MIN_SIZE);
    for (const tool of TOOLS) {
      const label = text.workspace.tools[tool];
      await page.locator("nav.ide-activity").getByRole("button", { name: label, exact: true }).click();
      await expect(page.locator(".ide-sidebar-header")).toHaveText(label);
      await expect(page.locator(".ide-sidebar-scroll > div").last()).not.toBeEmpty();
      await inspect(page, `workspace-tool-${tool}-${locale}-${MIN_SIZE.name}`, unanswered);
      // A long panel is captured again at its end, which the first picture leaves below the fold.
      const panel = page.locator(".ide-sidebar-scroll");
      if (await panel.evaluate((element) => element.scrollHeight > element.clientHeight + 1)) {
        await panel.evaluate((element) => element.scrollTo(0, element.scrollHeight));
        await inspect(page, `workspace-tool-${tool}-end-${locale}-${MIN_SIZE.name}`, unanswered);
        await panel.evaluate((element) => element.scrollTo(0, 0));
      }
    }
  });
}

type Scene = {
  name: string;
  screen: Screen;
  /** Opens the dialog or menu. */
  act: (page: Page, text: Texts) => Promise<void>;
};

/** The menu of the second project card, the one with the long name. */
async function openProjectMenu(page: Page, text: Texts) {
  await page.getByRole("button", { name: text.workspace.projectActions }).nth(1).click();
}

const SCENES: Scene[] = [
  {
    name: "projects-new",
    screen: PROJECTS,
    act: async (page, text) => {
      await page.getByRole("button", { name: text.workspace.newProject }).first().click();
      await expect(page.getByRole("dialog")).toBeVisible();
    },
  },
  {
    name: "projects-menu",
    screen: PROJECTS,
    act: async (page, text) => {
      await openProjectMenu(page, text);
      await expect(page.getByRole("button", { name: text.workspace.rename })).toBeVisible();
    },
  },
  {
    name: "projects-rename",
    screen: PROJECTS,
    act: async (page, text) => {
      await openProjectMenu(page, text);
      await page.getByRole("button", { name: text.workspace.rename }).click();
      await expect(page.getByRole("dialog")).toBeVisible();
    },
  },
  {
    name: "projects-delete",
    screen: PROJECTS,
    act: async (page, text) => {
      await openProjectMenu(page, text);
      await page.getByRole("button", { name: text.workspace.deleteProject }).click();
      await expect(page.getByRole("dialog")).toBeVisible();
    },
  },
  {
    name: "settings-reset",
    screen: SETTINGS,
    act: async (page, text) => {
      await page.getByRole("button", { name: text.settings.reset, exact: true }).click();
      await expect(page.getByRole("dialog")).toBeVisible();
    },
  },
  {
    // Below the chat assistant: analysis folded onto the chat's settings, and the image backends.
    name: "settings-analysis",
    screen: SETTINGS,
    act: async (page, text) => {
      const heading = page.getByRole("heading", { name: text.settings.analysisSection, exact: true });
      await heading.evaluate((element) => element.scrollIntoView({ block: "start" }));
      await expect(page.getByRole("checkbox", { name: text.settings.analysisSameAsAgent })).toBeChecked();
    },
  },
  {
    // The chat assistant's model list: OpenRouter's models with their prices.
    name: "settings-models",
    screen: SETTINGS,
    act: async (page, text) => {
      await page.getByRole("button", { name: text.settings.modelListShow }).first().click();
      await expect(page.getByRole("listbox")).toBeVisible();
    },
  },
  {
    name: "workspace-delete-message",
    screen: WORKSPACE,
    act: async (page, text) => {
      await page.getByRole("button", { name: text.agent.deleteMessage }).first().click();
      await expect(page.getByRole("dialog")).toBeVisible();
    },
  },
  {
    name: "workspace-image",
    screen: WORKSPACE,
    act: async (page) => {
      await page.locator(".ide-ref-thumb").first().click();
      await expect(page.getByRole("dialog")).toBeVisible();
    },
  },
];

// Dialogs and menus, at the smaller size.
for (const scene of SCENES) {
  for (const locale of LOCALES) {
    const name = `${scene.name}-${locale}-${MIN_SIZE.name}`;
    test(name, async ({ page }) => {
      const unanswered = await open(page, scene.screen, locale, MIN_SIZE);
      await scene.act(page, TEXTS[locale]);
      await inspect(page, name, unanswered);
    });
  }
}
