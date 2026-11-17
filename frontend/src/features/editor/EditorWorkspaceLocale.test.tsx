import { ThemeProvider } from "@mui/material/styles";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { kigTheme } from "../../app/theme";
import i18n from "../../i18n";
import { literalTranslations } from "../../i18n/literalTranslations";
import ja from "../../i18n/resources/ja";
import { editorTools } from "./components/EditorToolRail";
import { EditorWorkspace } from "./EditorWorkspace";

const pixiMocks = vi.hoisted(() => ({
  calculateRecipePreview: vi.fn(() => ({
    allLiquifyStrokes: [],
    displacementScale: { x: 0, y: 0 },
    detailRegionCount: 0,
    eyeMeshTransforms: [],
    eyeOffset: { x: 0, y: 0 },
    eyeScale: { x: 1, y: 1 },
    eyeSkew: 0,
    eyeTransformCount: 0,
    featureLiquifyStrokes: [],
    featureStrokeCount: 0,
    imageOffset: { x: 0, y: 0 },
    imageScale: { x: 1, y: 1 },
    imageSkew: { x: 0, y: 0 },
    jawScale: { x: 1, y: 1 },
    liquifyIntensity: 0,
    manualStrokeCount: 0,
    mouthMeshTransforms: [],
    mouthTransformCount: 0,
    strokeCount: 0,
  })),
  mountPixiStage: vi.fn(),
}));

vi.mock("./deformation/pixiStage", () => ({
  calculateRecipePreview: pixiMocks.calculateRecipePreview,
  mountPixiStage: pixiMocks.mountPixiStage,
}));

vi.mock("./deformation/animeLandmarkDetector", () => ({
  detectAnimeLandmarks: vi.fn().mockResolvedValue(null),
  warmupAnimeLandmarkDetector: vi.fn().mockResolvedValue(undefined),
}));

const HAN = /[㐀-鿿]/;
const KANA = /[぀-ヿ]/;

function collectStrings(value: unknown, out: Set<string>) {
  if (typeof value === "string") out.add(value.trim());
  else if (value && typeof value === "object") for (const child of Object.values(value)) collectStrings(child, out);
}

// Japanese UI text written only in kanji (保存, 液化…) is fine as long as it is one of our translations.
const japaneseStrings = new Set<string>();
collectStrings(ja, japaneseStrings);
collectStrings(literalTranslations.ja, japaneseStrings);

function visibleStrings(root: HTMLElement) {
  const strings: string[] = [];
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = node.textContent?.trim();
    if (text) strings.push(text);
  }
  for (const element of root.querySelectorAll("[aria-label], [placeholder], [title]")) {
    for (const attribute of ["aria-label", "placeholder", "title"]) {
      const text = element.getAttribute(attribute)?.trim();
      if (text) strings.push(text);
    }
  }
  return strings;
}

function untranslated(strings: string[], locale: "en" | "ja") {
  return strings.filter((text) => {
    if (!HAN.test(text)) return false;
    if (locale === "en") return true;
    if (KANA.test(text)) return false;
    // Strip interpolated parts (marker prefix, color codes) before looking the wording up.
    const wording = text
      .replace(/^landmark\s+/, "")
      .replace(/\s*#[0-9a-fA-F]{3,8}$/, "")
      .trim();
    return !japaneseStrings.has(text) && !japaneseStrings.has(wording);
  });
}

function renderEditor() {
  Object.defineProperty(window, "matchMedia", {
    value: vi.fn().mockImplementation((query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
  });
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  pixiMocks.mountPixiStage.mockResolvedValue({
    applyRecipe: vi.fn(),
    destroy: vi.fn(),
    exportImage: vi.fn(),
    setImageUrl: vi.fn().mockResolvedValue(undefined),
  });
  return render(
    <ThemeProvider theme={kigTheme}>
      <EditorWorkspace
        candidateIndex={2}
        imageHeight={720}
        imageUrl="/candidate.webp"
        imageWidth={1280}
        onLocalGenerate={vi.fn()}
        onRegenerate={vi.fn()}
        onSave={vi.fn()}
      />
    </ThemeProvider>,
  );
}

describe.each(["ja", "en"] as const)("EditorWorkspace in %s", (locale) => {
  beforeAll(async () => {
    await i18n.changeLanguage(locale);
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  afterAll(async () => {
    await i18n.changeLanguage("zh-CN");
  });

  it("sets the document language for the locale's font stack", () => {
    expect(document.documentElement.lang).toBe(locale);
  });

  it("leaves no Chinese text in any editor tool", async () => {
    renderEditor();
    await waitFor(() => expect(pixiMocks.mountPixiStage).toHaveBeenCalled());

    const leftovers = new Set<string>();
    for (const tool of editorTools) {
      const button = screen.queryByTestId(`editor-tool-${tool.key}`);
      if (!button) continue;
      fireEvent.click(button);
      for (const text of untranslated(visibleStrings(document.body), locale)) leftovers.add(`${tool.key}: ${text}`);
    }

    expect([...leftovers]).toEqual([]);
  });
});
