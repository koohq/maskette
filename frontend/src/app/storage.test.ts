import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  AGENT_OPEN_STORAGE_KEY,
  AGENT_WIDTH_STORAGE_KEY,
  LEGACY_AGENT_OPEN_STORAGE_KEY,
  LEGACY_AGENT_WIDTH_STORAGE_KEY,
  readStoredAgentOpen,
  readStoredAgentWidth,
} from "../features/workspace/agentWidth";
import { LEGACY_LOCALE_STORAGE_KEY, LOCALE_STORAGE_KEY, readStoredLocale } from "../i18n/locales";
import { readStored, writeStored } from "./storage";
import { LEGACY_THEME_STORAGE_KEY, readStoredThemeMode, THEME_STORAGE_KEY } from "./theme";

describe("stored UI preferences", () => {
  beforeEach(() => window.localStorage.clear());
  afterEach(() => {
    vi.restoreAllMocks();
    window.localStorage.clear();
  });

  it("names every key after Maskette", () => {
    for (const key of [THEME_STORAGE_KEY, AGENT_WIDTH_STORAGE_KEY, AGENT_OPEN_STORAGE_KEY, LOCALE_STORAGE_KEY]) {
      expect(key).toMatch(/^maskette\./);
    }
  });

  it("reads a value saved under the old key and moves it to the new key", () => {
    window.localStorage.setItem(LEGACY_THEME_STORAGE_KEY, "light");
    window.localStorage.setItem(LEGACY_AGENT_WIDTH_STORAGE_KEY, "512");
    window.localStorage.setItem(LEGACY_AGENT_OPEN_STORAGE_KEY, "0");
    window.localStorage.setItem(LEGACY_LOCALE_STORAGE_KEY, "ja");

    expect(readStoredThemeMode()).toBe("light");
    expect(readStoredAgentWidth(1440)).toBe(512);
    expect(readStoredAgentOpen()).toBe(false);
    expect(readStoredLocale()).toBe("ja");

    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe("light");
    expect(window.localStorage.getItem(AGENT_WIDTH_STORAGE_KEY)).toBe("512");
    expect(window.localStorage.getItem(AGENT_OPEN_STORAGE_KEY)).toBe("0");
    expect(window.localStorage.getItem(LOCALE_STORAGE_KEY)).toBe("ja");
    for (const legacy of [
      LEGACY_THEME_STORAGE_KEY,
      LEGACY_AGENT_WIDTH_STORAGE_KEY,
      LEGACY_AGENT_OPEN_STORAGE_KEY,
      LEGACY_LOCALE_STORAGE_KEY,
    ]) {
      expect(window.localStorage.getItem(legacy)).toBeNull();
    }
  });

  it("prefers the new key over a leftover old one", () => {
    window.localStorage.setItem("maskette.test.v1", "new");
    window.localStorage.setItem("kigcraft.test.v1", "old");
    expect(readStored("maskette.test.v1", "kigcraft.test.v1")).toBe("new");
    expect(window.localStorage.getItem("kigcraft.test.v1")).toBe("old");
  });

  it("returns null when neither key is saved", () => {
    expect(readStored("maskette.test.v1", "kigcraft.test.v1")).toBeNull();
    expect(readStored("maskette.test.v1")).toBeNull();
  });

  it("writes only the new key", () => {
    writeStored("maskette.test.v1", "1");
    expect(window.localStorage.getItem("maskette.test.v1")).toBe("1");
    expect(window.localStorage.length).toBe(1);
  });

  it("keeps the old value when the move cannot be written", () => {
    window.localStorage.setItem("kigcraft.test.v1", "old");
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new DOMException("Storage is full", "QuotaExceededError");
    });
    expect(readStored("maskette.test.v1", "kigcraft.test.v1")).toBe("old");
    expect(window.localStorage.getItem("kigcraft.test.v1")).toBe("old");
    expect(() => writeStored("maskette.test.v1", "1")).not.toThrow();
  });

  it("falls back to null when storage reads throw", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new DOMException("Storage is blocked", "SecurityError");
    });
    expect(readStored("maskette.test.v1", "kigcraft.test.v1")).toBeNull();
    expect(readStoredThemeMode()).toBe("dark");
    expect(readStoredAgentOpen()).toBe(true);
  });
});
