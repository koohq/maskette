import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import i18n from "../../i18n";
import { AUTO_EYE_SYMMETRY_STORAGE_KEY, readAutoEyeSymmetry } from "../workspace/autoFixPreferences";
import { AutoFixSection } from "./AutoFixSection";

describe("AutoFixSection", () => {
  beforeAll(async () => {
    await i18n.changeLanguage("ja");
  });

  beforeEach(() => {
    window.localStorage.clear();
  });

  afterEach(cleanup);

  it("fixes uneven eyes by default and remembers turning it off and on", () => {
    render(<AutoFixSection />);
    const toggle = screen.getByRole("checkbox", { name: "左右の目のずれを自動で直す" });

    expect(toggle).toBeChecked();
    expect(readAutoEyeSymmetry()).toBe(true);

    fireEvent.click(toggle);
    expect(toggle).not.toBeChecked();
    expect(window.localStorage.getItem(AUTO_EYE_SYMMETRY_STORAGE_KEY)).toBe("0");
    expect(readAutoEyeSymmetry()).toBe(false);

    fireEvent.click(toggle);
    expect(readAutoEyeSymmetry()).toBe(true);
  });

  it("starts from the saved choice", () => {
    window.localStorage.setItem(AUTO_EYE_SYMMETRY_STORAGE_KEY, "0");
    render(<AutoFixSection />);

    expect(screen.getByRole("checkbox", { name: "左右の目のずれを自動で直す" })).not.toBeChecked();
  });
});
