import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import type { ReactNode } from "react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import i18n from "../../i18n";
import { AboutSection } from "./AboutSection";
import { LicensesPage } from "./LicensesPage";
import type { Notices } from "./notices";

const NOTICES: Notices = {
  appVersion: "0.1.0",
  components: [
    {
      name: "crate-a",
      version: "1.2.3",
      group: "app",
      license: "MIT",
      url: "https://crates.io/crates/crate-a/1.2.3",
      texts: [0],
    },
    {
      name: "package-b",
      version: "4.5.6",
      group: "screen",
      license: "Apache-2.0",
      url: "https://www.npmjs.com/package/package-b/v/4.5.6",
      note: "A note on package-b.",
      texts: [1],
    },
    {
      name: "crate-c",
      version: "0.1.0",
      group: "app",
      license: "MIT",
      url: "https://crates.io/crates/crate-c/0.1.0",
      texts: [0],
    },
  ],
  texts: ["MIT License\n\nCopyright (c) Example Author", "Apache License\nVersion 2.0, January 2004"],
};

function respond(body: string, contentType: string, status = 200) {
  return new Response(body, { status, headers: { "Content-Type": contentType } });
}

function renderWith(children: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter>{children}</MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("about and third-party licenses", () => {
  beforeAll(async () => {
    await i18n.changeLanguage("ja");
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  it("says what Maskette is licensed under and links to the third-party licenses", () => {
    renderWith(<AboutSection />);
    expect(screen.getByRole("heading", { name: "Maskette について" })).toBeInTheDocument();
    expect(screen.getByText(/GNU GPL バージョン 3.*本ソフトウェアは無保証です。/)).toBeInTheDocument();
    expect(screen.getByText("ソースコード：https://github.com/koohq/maskette")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "オープンソースライセンス" })).toHaveAttribute("href", "/licenses");
  });

  it("lists the components by group and shows a license text when one is opened", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => respond(JSON.stringify(NOTICES), "application/json")),
    );
    renderWith(<LicensesPage />);

    const app = (await screen.findByRole("heading", { name: /アプリ本体（Rust）/ })).closest("section")!;
    expect(within(app).getByText("2 件")).toBeInTheDocument();
    const names = within(app)
      .getAllByRole("listitem")
      .map((item) => item.querySelector("summary span")?.textContent);
    expect(names).toEqual(["crate-a", "crate-c"]);

    const screenGroup = screen.getByRole("heading", { name: /画面（JavaScript）/ }).closest("section")!;
    expect(within(screenGroup).queryByText(/Apache License/)).not.toBeInTheDocument();
    const summary = within(screenGroup).getByText("package-b").closest("summary")!;
    const details = summary.closest("details")!;
    details.open = true;
    fireEvent(details, new Event("toggle"));
    expect(await within(screenGroup).findByText(/Apache License/)).toBeInTheDocument();
    expect(within(screenGroup).getByText("A note on package-b.")).toBeInTheDocument();

    expect(screen.getByRole("link", { name: "テキストで保存" })).toHaveAttribute("download", "third-party-notices.txt");
    expect(screen.queryByRole("heading", { name: /インストーラー/ })).not.toBeInTheDocument();
  });

  it("says the list is missing when the app answers with its own page instead", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => respond("<!doctype html><html></html>", "text/html")),
    );
    renderWith(<LicensesPage />);
    expect(await screen.findByText(/ライセンス情報が見つかりません/)).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "テキストで保存" })).not.toBeInTheDocument();
  });
});
