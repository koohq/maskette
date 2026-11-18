import { describe, expect, it } from "vitest";
import i18n from "../../i18n";
import { formatPrice } from "./ModelField";

describe("formatPrice", () => {
  it("shows each kind of price compactly", async () => {
    await i18n.changeLanguage("ja");
    const t = i18n.t.bind(i18n);
    expect(formatPrice(t, { unit: "token", input: 0.3, output: 2.5 })).toBe("$0.3 / $2.5");
    expect(formatPrice(t, { unit: "token", input: 0, output: 0 })).toBe("無料");
    expect(formatPrice(t, { unit: "image", input: null, output: 0.04 })).toBe("$0.04 / 枚");
    expect(formatPrice(t, { unit: "token", input: null, output: 30 })).toBe("$30 / 100 万トークン");
    expect(formatPrice(t, { unit: "token", input: 0.0375, output: 15.123 })).toBe("$0.0375 / $15.1");
    expect(formatPrice(t, { unit: "image", input: null, output: null })).toBe("");
    await i18n.changeLanguage("zh-CN");
  });
});
