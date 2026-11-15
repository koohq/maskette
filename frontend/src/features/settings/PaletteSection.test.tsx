import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { Providers } from "../../app/providers";
import { idePalettes, PALETTE_STORAGE_KEY } from "../../app/theme";
import i18n from "../../i18n";
import { PaletteSection } from "./PaletteSection";

function renderSection() {
  return render(
    <Providers>
      <PaletteSection />
    </Providers>,
  );
}

describe("PaletteSection", () => {
  beforeAll(async () => {
    await i18n.changeLanguage("ja");
  });

  afterEach(() => {
    cleanup();
    window.localStorage.clear();
    document.documentElement.removeAttribute("style");
  });

  it("starts on the default palette and marks it", () => {
    renderSection();
    expect(screen.getByRole("radio", { name: "蜜柑（オレンジ）" })).toBeChecked();
    expect(screen.getByText("既定")).toBeInTheDocument();
    expect(document.documentElement.dataset["palette"]).toBe("mikan");
    expect(document.documentElement.style.getPropertyValue("--ide-accent")).toBe(idePalettes.mikan.dark.accent);
  });

  it("switches the app colours at once and keeps the choice", () => {
    renderSection();
    fireEvent.click(screen.getByRole("radio", { name: "藍（以前の青）" }));
    expect(screen.getByRole("radio", { name: "藍（以前の青）" })).toBeChecked();
    expect(document.documentElement.dataset["palette"]).toBe("ai");
    expect(document.documentElement.style.getPropertyValue("--ide-bg")).toBe(idePalettes.ai.dark.background);
    expect(window.localStorage.getItem(PALETTE_STORAGE_KEY)).toBe("ai");
  });

  it("opens on the saved palette", () => {
    window.localStorage.setItem(PALETTE_STORAGE_KEY, "wakaba");
    renderSection();
    expect(screen.getByRole("radio", { name: "若葉（緑）" })).toBeChecked();
    expect(document.documentElement.style.getPropertyValue("--ide-accent")).toBe(idePalettes.wakaba.dark.accent);
  });
});
