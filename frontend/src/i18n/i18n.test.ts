import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "./index";
import { translateLiteralString } from "./literalTranslations";
import {
  DEFAULT_LOCALE,
  LOCALE_STORAGE_KEY,
  detectBrowserLocale,
  normalizeLocale,
  readStoredLocale,
  SUPPORTED_LOCALES,
  writeStoredLocale,
} from "./locales";

describe("i18n", () => {
  beforeEach(() => {
    window.localStorage.clear();
    setBrowserLanguages(["zh-CN"]);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    window.localStorage.clear();
  });

  it("normalizes unsupported locale values to Chinese", () => {
    expect(normalizeLocale("zh-CN")).toBe("zh-CN");
    expect(normalizeLocale("en")).toBe("en");
    expect(normalizeLocale("en-US")).toBe("en");
    expect(normalizeLocale("ja")).toBe("ja");
    expect(normalizeLocale("ja-JP")).toBe("ja");
    expect(normalizeLocale("zh-TW")).toBe("zh-CN");
    expect(normalizeLocale("fr")).toBe(DEFAULT_LOCALE);
    expect(normalizeLocale("")).toBe(DEFAULT_LOCALE);
    expect(normalizeLocale(null)).toBe(DEFAULT_LOCALE);
  });

  it("declares the selectable language menu locales", () => {
    expect(SUPPORTED_LOCALES.map((locale) => locale.value)).toEqual(["zh-CN", "en", "ja"]);
    expect(SUPPORTED_LOCALES.map((locale) => locale.label)).toEqual(["中文", "English", "日本語"]);
  });

  it("uses English resources when English is selected", async () => {
    await i18n.changeLanguage("en");
    expect(i18n.t("workflow.steps.detail")).toBe("Confirm Details");
    expect(i18n.t("detailConfirmation.analysisStatus.title")).toBe("Analyzing Details");
    expect(i18n.t("generation.waitingForProgress")).toBe("Waiting for generation updates");
  });

  it("uses Japanese resources when Japanese is selected", async () => {
    await i18n.changeLanguage("ja");
    expect(i18n.t("workflow.steps.detail")).toBe("細部を確認");
    expect(i18n.t("detailConfirmation.analysisStatus.title")).toBe("細部を分析中");
    expect(i18n.t("generation.waitingForProgress")).toBe("生成状況の更新を待っています");
  });

  it("stores and reads supported locale values", () => {
    writeStoredLocale("ja");

    expect(window.localStorage.getItem(LOCALE_STORAGE_KEY)).toBe("ja");
    expect(readStoredLocale()).toBe("ja");
  });

  it("uses the browser language when no locale is stored", () => {
    setBrowserLanguages(["ja-JP", "en-US"]);

    expect(detectBrowserLocale()).toBe("ja");
    expect(readStoredLocale()).toBe("ja");
  });

  it("uses the browser language when the stored locale is invalid", () => {
    setBrowserLanguages(["en-US"]);
    window.localStorage.setItem(LOCALE_STORAGE_KEY, "fr");

    expect(readStoredLocale()).toBe("en");
  });

  it("uses the browser language when localStorage reads throw", () => {
    setBrowserLanguages(["ja-JP"]);
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new DOMException("Storage is blocked", "SecurityError");
    });

    expect(readStoredLocale()).toBe("ja");
  });

  it("follows the selected locale in the document language", async () => {
    await i18n.changeLanguage("ja");
    expect(document.documentElement.lang).toBe("ja");
    await i18n.changeLanguage("en-US");
    expect(document.documentElement.lang).toBe("en");
    await i18n.changeLanguage("zh-CN");
    expect(document.documentElement.lang).toBe("zh-CN");
  });

  it("translates literal strings that carry interpolated values", () => {
    const auditIntro = "查看当前后端进程内的生图任务、提交参考图和输出图片。列表支持滚动，共 12 条。";
    expect(translateLiteralString(auditIntro, "ja")).toBe(
      "現在のバックエンドプロセス内の生成タスク、送信した参考画像、出力画像を表示します。リストはスクロールできます。全 12 件。",
    );
    expect(translateLiteralString("候选 2 编辑画布", "ja")).toBe("候補 2 編集キャンバス");
    expect(translateLiteralString("标注颜色 #ef4444", "en")).toBe("Annotation color #ef4444");
    expect(translateLiteralString("颜色 #ef4444", "ja")).toBe("色 #ef4444");
    expect(translateLiteralString("粗细减少", "ja")).toBe("太さを減らす");
    expect(translateLiteralString("局部生成提交失败。", "en")).toBe("Failed to submit local generation.");
    expect(translateLiteralString("粗细", "zh-CN")).toBe("粗细");
  });

  it("ignores localStorage write failures", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new DOMException("Storage is blocked", "SecurityError");
    });

    expect(() => writeStoredLocale("ja")).not.toThrow();
  });
});

function setBrowserLanguages(languages: string[]) {
  Object.defineProperty(navigator, "languages", {
    configurable: true,
    get: () => languages,
  });
  Object.defineProperty(navigator, "language", {
    configurable: true,
    get: () => languages[0] ?? "zh-CN",
  });
}
