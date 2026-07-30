import { describe, expect, it } from "vitest";
import { preferredLocale, translate } from "./i18n";

describe("localization", () => {
  it("selects Brazilian Portuguese for Portuguese system locales", () => {
    expect(preferredLocale("pt-BR")).toBe("pt-BR");
    expect(preferredLocale("pt-PT")).toBe("pt-BR");
  });

  it("falls back to English and interpolates localized values", () => {
    expect(preferredLocale("en-US")).toBe("en");
    expect(translate("pt-BR", "removal.title", { name: "Atelier" })).toBe("Remover Atelier?");
  });
});
