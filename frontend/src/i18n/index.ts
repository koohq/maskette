import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { DEFAULT_LOCALE, normalizeLocale, readStoredLocale } from "./locales";
import en from "./resources/en";
import ja from "./resources/ja";
import zhCN from "./resources/zh-CN";

// <html lang> picks the locale's font stack (styles/index.css) and tells the
// browser which glyph forms to use for Han characters.
export function syncDocumentLanguage(language: string | undefined) {
  if (typeof document === "undefined") return;
  document.documentElement.lang = normalizeLocale(language);
}

const initialLocale = readStoredLocale();
syncDocumentLanguage(initialLocale);
i18n.on("languageChanged", syncDocumentLanguage);

void i18n.use(initReactI18next).init({
  fallbackLng: DEFAULT_LOCALE,
  interpolation: {
    escapeValue: false,
  },
  lng: initialLocale,
  resources: {
    en: {
      translation: en,
    },
    ja: {
      translation: ja,
    },
    "zh-CN": {
      translation: zhCN,
    },
  },
});

export default i18n;
