import { createTheme, type Theme } from "@mui/material/styles";
import { readStored } from "./storage";

export type ThemeMode = "dark" | "light";

export const THEME_STORAGE_KEY = "maskette.theme.v1";
export const LEGACY_THEME_STORAGE_KEY = "kigcraft.theme.v1";
export const PALETTE_STORAGE_KEY = "maskette.palette.v1";

/** Colour sets the user can pick in Settings. The first one is the default. */
export const PALETTE_IDS = ["mikan", "wakaba", "ai"] as const;
export type PaletteId = (typeof PALETTE_IDS)[number];
export const DEFAULT_PALETTE: PaletteId = "mikan";

export type IdeColors = {
  accent: string;
  accentHover: string;
  accentContrast: string;
  accentSoft: string;
  background: string;
  panel: string;
  panelRaised: string;
  activity: string;
  border: string;
  borderStrong: string;
  text: string;
  textMuted: string;
  textFaint: string;
  hover: string;
  input: string;
  canvas: string;
  canvasCheck: string;
  danger: string;
  success: string;
  warning: string;
  shadow: string;
};

// Warm greys shared by the palettes taken from the logo (mandarin hair, green eyes).
const warmDark = {
  background: "#1a1817",
  panel: "#211f1d",
  panelRaised: "#2a2725",
  activity: "#141211",
  border: "#33302d",
  borderStrong: "#46413d",
  text: "#eee9e4",
  textMuted: "#a39c96",
  textFaint: "#716a64",
  hover: "#2c2926",
  input: "#141211",
  canvas: "#100f0e",
  canvasCheck: "#1a1817",
  danger: "#f26d6d",
  shadow: "0 10px 30px rgba(0, 0, 0, 0.4)",
} as const;

const warmLight = {
  background: "#f7f5f2",
  panel: "#ffffff",
  panelRaised: "#f0ede8",
  activity: "#ece8e2",
  border: "#e3ded6",
  borderStrong: "#cfc8be",
  text: "#2a211b",
  textMuted: "#6b6158",
  textFaint: "#8c8278",
  hover: "#f2ede6",
  input: "#ffffff",
  canvas: "#ece8e2",
  canvasCheck: "#e3ded6",
  danger: "#c93c3c",
  shadow: "0 10px 30px rgba(42, 33, 27, 0.12)",
} as const;

/**
 * Every colour of the IDE theme, per palette and mode. This is the single
 * source: the provider writes the chosen set onto <html> as the `--ide-*`
 * CSS variables (see `ideCssVariables`), and styles/index.css only repeats
 * the default palette so the first paint has colours (a test keeps the two
 * equal). Body text and text on the accent meet WCAG AA in every set.
 */
export const idePalettes: Record<PaletteId, Record<ThemeMode, IdeColors>> = {
  // Mandarin: the hair of the mascot. The default.
  mikan: {
    dark: {
      ...warmDark,
      accent: "#f59a4a",
      accentHover: "#f8ad6b",
      accentContrast: "#2a1404",
      accentSoft: "rgba(245, 154, 74, 0.16)",
      success: "#43c38c",
      warning: "#e5b454",
    },
    light: {
      ...warmLight,
      accent: "#b54d0f",
      accentHover: "#9a410c",
      accentContrast: "#ffffff",
      accentSoft: "rgba(181, 77, 15, 0.1)",
      success: "#1b7a52",
      warning: "#a86b12",
    },
  },
  // Young leaf: the eyes of the mascot. Success turns blue so it does not read as the accent.
  wakaba: {
    dark: {
      ...warmDark,
      accent: "#3fbf80",
      accentHover: "#5ccd94",
      accentContrast: "#05200f",
      accentSoft: "rgba(63, 191, 128, 0.16)",
      success: "#5bb8e6",
      warning: "#f2a65a",
    },
    light: {
      ...warmLight,
      accent: "#1c7f4f",
      accentHover: "#166a41",
      accentContrast: "#ffffff",
      accentSoft: "rgba(28, 127, 79, 0.1)",
      success: "#1f6fa8",
      warning: "#b25d0c",
    },
  },
  // Indigo: the blue on navy the app had before its own logo (light-mode status colours darkened to meet AA).
  ai: {
    dark: {
      accent: "#2e95f8",
      accentHover: "#52a8fa",
      accentContrast: "#04122a",
      accentSoft: "rgba(46, 149, 248, 0.16)",
      background: "#0a1426",
      panel: "#0e1a30",
      panelRaised: "#14223b",
      activity: "#071022",
      border: "#1e2d48",
      borderStrong: "#2b3d5e",
      text: "#e3eaf5",
      textMuted: "#8796b0",
      textFaint: "#566684",
      hover: "#172741",
      input: "#071022",
      canvas: "#060d1b",
      canvasCheck: "#0a1426",
      danger: "#f26d6d",
      success: "#43c38c",
      warning: "#e5b454",
      shadow: "0 10px 30px rgba(0, 0, 0, 0.4)",
    },
    light: {
      accent: "#116ced",
      accentHover: "#0b59c8",
      accentContrast: "#ffffff",
      accentSoft: "rgba(17, 108, 237, 0.1)",
      background: "#f4f7fc",
      panel: "#ffffff",
      panelRaised: "#edf2f9",
      activity: "#e6edf7",
      border: "#d9e2ef",
      borderStrong: "#c1cee1",
      text: "#0f1f3d",
      textMuted: "#5a6a86",
      textFaint: "#7d8aa1",
      hover: "#e8eff9",
      input: "#ffffff",
      canvas: "#e3eaf4",
      canvasCheck: "#dae2ee",
      danger: "#c93c3c",
      success: "#1d7f57",
      warning: "#b7791f",
      shadow: "0 10px 30px rgba(7, 22, 48, 0.12)",
    },
  },
};

const cssNames: Record<keyof IdeColors, string> = {
  accent: "--ide-accent",
  accentHover: "--ide-accent-hover",
  accentContrast: "--ide-accent-contrast",
  accentSoft: "--ide-accent-soft",
  background: "--ide-bg",
  panel: "--ide-panel",
  panelRaised: "--ide-panel-raised",
  activity: "--ide-activity",
  border: "--ide-border",
  borderStrong: "--ide-border-strong",
  text: "--ide-text",
  textMuted: "--ide-text-muted",
  textFaint: "--ide-text-faint",
  hover: "--ide-hover",
  input: "--ide-input",
  canvas: "--ide-canvas",
  canvasCheck: "--ide-canvas-check",
  danger: "--ide-danger",
  success: "--ide-success",
  warning: "--ide-warning",
  shadow: "--ide-shadow",
};

/** The `--ide-*` CSS variables for one palette and mode. */
export function ideCssVariables(palette: PaletteId, mode: ThemeMode): Record<string, string> {
  const colors = idePalettes[palette][mode];
  return Object.fromEntries((Object.keys(cssNames) as (keyof IdeColors)[]).map((key) => [cssNames[key], colors[key]]));
}

// The actual font stack lives in styles/index.css and switches with <html lang>.
export const uiFontFamily = "var(--ui-font-family)";

export function readStoredThemeMode(): ThemeMode {
  return readStored(THEME_STORAGE_KEY, LEGACY_THEME_STORAGE_KEY) === "light" ? "light" : "dark";
}

export function isPaletteId(value: unknown): value is PaletteId {
  return typeof value === "string" && (PALETTE_IDS as readonly string[]).includes(value);
}

export function readStoredPalette(): PaletteId {
  const stored = readStored(PALETTE_STORAGE_KEY);
  return isPaletteId(stored) ? stored : DEFAULT_PALETTE;
}

export function createKigTheme(mode: ThemeMode, palette: PaletteId = DEFAULT_PALETTE): Theme {
  const p = idePalettes[palette][mode];
  return createTheme({
    palette: {
      mode,
      primary: { main: p.accent, dark: p.accentHover, light: p.accentHover, contrastText: p.accentContrast },
      secondary: { main: p.textMuted, contrastText: p.background },
      error: { main: p.danger },
      warning: { main: p.warning },
      success: { main: p.success },
      background: { default: p.background, paper: p.panel },
      text: { primary: p.text, secondary: p.textMuted },
      divider: p.border,
    },
    shape: { borderRadius: 6 },
    typography: {
      fontFamily: uiFontFamily,
      fontSize: 13,
      button: { fontWeight: 600, letterSpacing: 0, textTransform: "none" },
      h1: { fontSize: "1.6rem", fontWeight: 700 },
      h2: { fontSize: "1.35rem", fontWeight: 700 },
      h3: { fontSize: "1.1rem", fontWeight: 650 },
      h4: { fontSize: "0.95rem", fontWeight: 650 },
      h5: { fontSize: "0.875rem", fontWeight: 650 },
    },
    components: {
      MuiButton: {
        defaultProps: { disableElevation: true, size: "small" },
        styleOverrides: { root: { borderRadius: 6, minWidth: 0, whiteSpace: "nowrap" } },
      },
      MuiIconButton: {
        styleOverrides: {
          root: { borderRadius: 6, color: p.textMuted, "&:hover": { backgroundColor: p.hover, color: p.text } },
        },
      },
      MuiPaper: { styleOverrides: { root: { backgroundImage: "none" } } },
      MuiMenu: {
        styleOverrides: {
          paper: {
            backgroundColor: p.panelRaised,
            border: `1px solid ${p.border}`,
            boxShadow: "0 8px 24px rgba(0,0,0,0.28)",
          },
        },
      },
      MuiSlider: {
        styleOverrides: {
          root: { color: p.accent, height: 3, padding: "10px 0" },
          rail: { backgroundColor: p.borderStrong, opacity: 1 },
          thumb: {
            backgroundColor: p.panel,
            border: `2px solid ${p.accent}`,
            height: 12,
            width: 12,
            boxShadow: "none",
            "&:hover, &.Mui-focusVisible": { boxShadow: `0 0 0 5px ${p.accent}33` },
          },
          track: { border: 0 },
        },
      },
      MuiTooltip: {
        styleOverrides: {
          tooltip: { backgroundColor: p.panelRaised, border: `1px solid ${p.border}`, color: p.text, fontSize: 12 },
        },
      },
      MuiAlert: {
        styleOverrides: { root: { border: `1px solid ${p.border}`, backgroundColor: p.panelRaised, color: p.text } },
      },
      MuiLinearProgress: {
        styleOverrides: { root: { backgroundColor: p.border, borderRadius: 2 }, bar: { backgroundColor: p.accent } },
      },
      MuiTab: { styleOverrides: { root: { minHeight: 36, textTransform: "none" } } },
      MuiTextField: {
        styleOverrides: {
          root: {
            "& .MuiFilledInput-root": {
              backgroundColor: p.input,
              border: `1px solid ${p.border}`,
              borderRadius: 6,
              "&:before, &:after": { display: "none" },
              "&:hover": { backgroundColor: p.input, borderColor: p.borderStrong },
              "&.Mui-focused": { backgroundColor: p.input, borderColor: p.accent },
            },
          },
        },
      },
    },
  });
}

/** Dark theme in the default palette, kept for tests and legacy imports. */
export const kigTheme = createKigTheme("dark");
