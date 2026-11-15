import { readFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  createKigTheme,
  DEFAULT_PALETTE,
  ideCssVariables,
  idePalettes,
  PALETTE_IDS,
  PALETTE_STORAGE_KEY,
  readStoredPalette,
  type ThemeMode,
} from "./theme";

describe("createKigTheme", () => {
  it("builds dark and light IDE themes from the default palette", () => {
    const dark = createKigTheme("dark");
    const light = createKigTheme("light");
    expect(dark.palette.mode).toBe("dark");
    expect(dark.palette.primary.main).toBe(idePalettes[DEFAULT_PALETTE].dark.accent);
    expect(dark.palette.background.default).toBe(idePalettes[DEFAULT_PALETTE].dark.background);
    expect(light.palette.mode).toBe("light");
    expect(light.palette.background.paper).toBe(idePalettes[DEFAULT_PALETTE].light.panel);
  });

  it("uses the colours of the palette it is given", () => {
    const theme = createKigTheme("light", "wakaba");
    expect(theme.palette.primary.main).toBe(idePalettes.wakaba.light.accent);
    expect(theme.palette.success.main).toBe(idePalettes.wakaba.light.success);
  });

  it("keeps compact Material UI defaults", () => {
    expect(createKigTheme("dark").components?.MuiButton?.defaultProps).toMatchObject({ disableElevation: true });
  });
});

describe("palettes", () => {
  it("starts with the mandarin palette taken from the logo", () => {
    expect(DEFAULT_PALETTE).toBe("mikan");
    expect(PALETTE_IDS[0]).toBe(DEFAULT_PALETTE);
  });

  it("maps every colour to its --ide-* variable", () => {
    const vars = ideCssVariables("ai", "dark");
    expect(vars["--ide-accent"]).toBe(idePalettes.ai.dark.accent);
    expect(vars["--ide-bg"]).toBe(idePalettes.ai.dark.background);
    expect(vars["--ide-shadow"]).toBe(idePalettes.ai.dark.shadow);
    expect(Object.keys(vars)).toHaveLength(Object.keys(idePalettes.ai.dark).length);
  });

  it("keeps every palette readable (WCAG contrast)", () => {
    const luminance = (hex: string) => {
      const channels = [1, 3, 5].map((i) => Number.parseInt(hex.slice(i, i + 2), 16) / 255);
      const [r = 0, g = 0, b = 0] = channels.map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
      return 0.2126 * r + 0.7152 * g + 0.0722 * b;
    };
    const contrast = (a: string, b: string) => {
      const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
      return (hi + 0.05) / (lo + 0.05);
    };
    for (const id of PALETTE_IDS) {
      for (const mode of ["dark", "light"] as const) {
        const c = idePalettes[id][mode];
        const where = `${id} ${mode}`;
        expect(contrast(c.text, c.background), `${where} text`).toBeGreaterThanOrEqual(4.5);
        expect(contrast(c.textMuted, c.background), `${where} muted text`).toBeGreaterThanOrEqual(4.5);
        expect(contrast(c.textFaint, c.background), `${where} faint text`).toBeGreaterThanOrEqual(3);
        expect(contrast(c.accentContrast, c.accent), `${where} text on accent`).toBeGreaterThanOrEqual(4.5);
        expect(contrast(c.accentContrast, c.accentHover), `${where} text on hovered accent`).toBeGreaterThanOrEqual(
          4.5,
        );
        expect(contrast(c.accent, c.panel), `${where} accent text`).toBeGreaterThanOrEqual(4.5);
        expect(contrast(c.danger, c.panel), `${where} danger text`).toBeGreaterThanOrEqual(4.5);
        expect(contrast(c.success, c.panel), `${where} success text`).toBeGreaterThanOrEqual(4.5);
        expect(contrast(c.warning, c.panel), `${where} warning`).toBeGreaterThanOrEqual(3);
      }
    }
  });

  it("repeats the default palette in styles/index.css for the first paint", () => {
    // `import.meta.url` is not a file URL under jsdom; Vitest gives the test file's folder as `import.meta.dirname`.
    const css = readFileSync(join(import.meta.dirname, "../styles/index.css"), "utf8");
    const block = (selector: RegExp) => {
      const body = css.match(selector)?.[1] ?? "";
      return Object.fromEntries([...body.matchAll(/(--ide-[\w-]+):\s*([^;]+);/g)].map((m) => [m[1], m[2]?.trim()]));
    };
    const cssModes: Record<ThemeMode, Record<string, string | undefined>> = {
      dark: block(/:root,\s*:root\[data-theme="dark"\]\s*\{([^}]*)\}/),
      light: block(/:root\[data-theme="light"\]\s*\{([^}]*)\}/),
    };
    for (const mode of ["dark", "light"] as const) {
      expect(cssModes[mode]).toEqual(ideCssVariables(DEFAULT_PALETTE, mode));
    }
  });
});

describe("readStoredPalette", () => {
  afterEach(() => window.localStorage.clear());

  it("falls back to the default when nothing or something unknown is saved", () => {
    expect(readStoredPalette()).toBe(DEFAULT_PALETTE);
    window.localStorage.setItem(PALETTE_STORAGE_KEY, "neon");
    expect(readStoredPalette()).toBe(DEFAULT_PALETTE);
  });

  it("returns a saved palette", () => {
    window.localStorage.setItem(PALETTE_STORAGE_KEY, "ai");
    expect(readStoredPalette()).toBe("ai");
  });
});
