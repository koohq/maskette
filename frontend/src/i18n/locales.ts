import { readStored, writeStored } from "../app/storage";

export const DEFAULT_LOCALE = "zh-CN" as const;
export const LOCALE_STORAGE_KEY = "maskette.locale.v1";
export const LEGACY_LOCALE_STORAGE_KEY = "kig-preview.locale.v1";

export const SUPPORTED_LOCALES = [
  { value: "zh-CN", label: "中文" },
  { value: "en", label: "English" },
  { value: "ja", label: "日本語" },
] as const;

export type AppLocale = (typeof SUPPORTED_LOCALES)[number]["value"];

const supportedLocaleValues = new Set<string>(SUPPORTED_LOCALES.map((locale) => locale.value));

export function normalizeLocale(value: unknown): AppLocale {
  return resolveSupportedLocale(value) ?? DEFAULT_LOCALE;
}

export function detectBrowserLocale(): AppLocale {
  if (typeof navigator === "undefined") return DEFAULT_LOCALE;

  const browserLanguages =
    Array.isArray(navigator.languages) && navigator.languages.length > 0 ? navigator.languages : [navigator.language];
  for (const language of browserLanguages) {
    const locale = resolveSupportedLocale(language);
    if (locale) return locale;
  }

  return DEFAULT_LOCALE;
}

export function readStoredLocale(): AppLocale {
  return resolveSupportedLocale(readStored(LOCALE_STORAGE_KEY, LEGACY_LOCALE_STORAGE_KEY)) ?? detectBrowserLocale();
}

export function writeStoredLocale(locale: AppLocale) {
  writeStored(LOCALE_STORAGE_KEY, locale);
}

function resolveSupportedLocale(value: unknown): AppLocale | null {
  if (typeof value !== "string") return null;
  if (supportedLocaleValues.has(value)) return value as AppLocale;

  const normalized = value.toLowerCase();
  if (normalized.startsWith("zh")) return "zh-CN";
  if (normalized.startsWith("en")) return "en";
  if (normalized.startsWith("ja")) return "ja";
  return null;
}
