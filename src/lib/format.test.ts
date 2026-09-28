import i18next from "i18next";
import { afterEach, describe, expect, it } from "vitest";
import { formatBytes, formatCount, formatNumber, formatParams } from "./format";

describe("números no formato do idioma", () => {
  afterEach(() => i18next.changeLanguage(undefined));

  it("português usa vírgula decimal e ponto de milhar", async () => {
    await i18next.init({ lng: "pt-BR", resources: {} });
    expect(formatNumber(1.5, 1)).toBe("1,5");
    expect(formatBytes(1.5 * 2 ** 30)).toBe("1,5 GB");
    expect(formatBytes(1234 * 2 ** 30)).toBe("1.234 GB");
    expect(formatCount(48_320)).toBe("48,3k");
    expect(formatParams(7.1e9)).toBe("7,1B");
  });

  it("inglês usa ponto decimal", async () => {
    await i18next.init({ lng: "en", resources: {} });
    expect(formatBytes(1.5 * 2 ** 30)).toBe("1.5 GB");
    expect(formatCount(1_250_000)).toBe("1.3M");
    expect(formatBytes(700 * 2 ** 20)).toBe("700 MB");
  });
});
