import { cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import i18n from "../../i18n";
import { HomeLink } from "./HomeLink";

describe("HomeLink", () => {
  beforeAll(async () => {
    await i18n.changeLanguage("ja");
  });

  afterEach(cleanup);

  it("is one link to the project list that holds the arrow and the logo", () => {
    render(
      <MemoryRouter>
        <HomeLink />
      </MemoryRouter>,
    );

    const link = screen.getByRole("link", { name: "全てのプロジェクト" });
    expect(link.getAttribute("href")).toBe("/");
    const logo = link.querySelector("img");
    expect(logo?.getAttribute("src")).toBe("/logo-head.png");
    // The link's label names it; the logo inside adds nothing for screen readers.
    expect(logo?.getAttribute("alt")).toBe("");
  });
});
