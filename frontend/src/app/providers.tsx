import { CssBaseline } from "@mui/material";
import { ThemeProvider } from "@mui/material/styles";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createContext, type ReactNode, useContext, useLayoutEffect, useMemo, useState } from "react";
import "../i18n";
import { writeStored } from "./storage";
import {
  createKigTheme,
  DEFAULT_PALETTE,
  ideCssVariables,
  PALETTE_STORAGE_KEY,
  type PaletteId,
  readStoredPalette,
  readStoredThemeMode,
  THEME_STORAGE_KEY,
  type ThemeMode,
} from "./theme";

const queryClient = new QueryClient();

type ThemeModeContextValue = { mode: ThemeMode; toggle: () => void };

const ThemeModeContext = createContext<ThemeModeContextValue>({ mode: "dark", toggle: () => undefined });

export function useThemeMode() {
  return useContext(ThemeModeContext);
}

type PaletteContextValue = { palette: PaletteId; setPalette: (palette: PaletteId) => void };

const PaletteContext = createContext<PaletteContextValue>({ palette: DEFAULT_PALETTE, setPalette: () => undefined });

export function usePalette() {
  return useContext(PaletteContext);
}

export function Providers({ children }: { children: ReactNode }) {
  const [mode, setMode] = useState<ThemeMode>(readStoredThemeMode);
  const [palette, setPalette] = useState<PaletteId>(readStoredPalette);
  const theme = useMemo(() => createKigTheme(mode, palette), [mode, palette]);

  // Before paint, so a saved palette never flashes the default colours.
  useLayoutEffect(() => {
    const root = document.documentElement;
    root.dataset["theme"] = mode;
    root.dataset["palette"] = palette;
    for (const [name, value] of Object.entries(ideCssVariables(palette, mode))) {
      root.style.setProperty(name, value);
    }
    writeStored(THEME_STORAGE_KEY, mode);
    writeStored(PALETTE_STORAGE_KEY, palette);
  }, [mode, palette]);

  const modeValue = useMemo(
    () => ({ mode, toggle: () => setMode((current) => (current === "dark" ? "light" : "dark")) }),
    [mode],
  );
  const paletteValue = useMemo(() => ({ palette, setPalette }), [palette]);

  return (
    <ThemeModeContext.Provider value={modeValue}>
      <PaletteContext.Provider value={paletteValue}>
        <ThemeProvider theme={theme}>
          <CssBaseline />
          <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
        </ThemeProvider>
      </PaletteContext.Provider>
    </ThemeModeContext.Provider>
  );
}
