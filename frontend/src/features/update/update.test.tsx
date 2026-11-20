import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "../../i18n";
import { UpdateBanner } from "./UpdateBanner";
import { UpdateSection } from "./UpdateSection";
import { CHECK_ON_LAUNCH_STORAGE_KEY, resetDismissedUpdate } from "./updates";

const RELEASE = { version: "0.9.0", notes: "目の拡大図を直しました。", published_at: 1_790_000_000 };
const IDLE = { supported: true, current_version: "0.1.0", install: { phase: "idle" } };

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
}

type Routes = {
  check?: () => Response;
  status?: () => Response;
  install?: () => Response;
  projects?: unknown[];
};

/** A stand-in backend: answers by path, and records every call as "METHOD /path". */
function stubBackend(routes: Routes = {}) {
  const calls: { call: string; body: unknown }[] = [];
  const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
    const path = new URL(url, "http://127.0.0.1").pathname;
    const method = init?.method ?? "GET";
    calls.push({ call: `${method} ${path}`, body: init?.body ? JSON.parse(String(init.body)) : undefined });
    if (path === "/api/update/check") return routes.check?.() ?? json({ ...IDLE, update: RELEASE });
    if (path === "/api/update/install") return routes.install?.() ?? json(IDLE, 202);
    if (path === "/api/update") return routes.status?.() ?? json(IDLE);
    if (path === "/api/agent/conversations") return json(routes.projects ?? []);
    return json({ detail: "Not Found" }, 404);
  });
  vi.stubGlobal("fetch", fetchMock);
  return calls;
}

function renderWithClient(children: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={client}>{children}</QueryClientProvider>);
}

describe("updates", () => {
  beforeAll(async () => {
    await i18n.changeLanguage("ja");
  });

  beforeEach(() => {
    window.localStorage.clear();
    window.__MASKETTE_BACKEND__ = { baseUrl: "http://127.0.0.1:4321", token: "test-token" };
  });

  afterEach(() => {
    cleanup();
    delete window.__MASKETTE_BACKEND__;
    resetDismissedUpdate();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  describe("UpdateBanner", () => {
    it("announces a newer version and installs it after the user confirms", async () => {
      let installed = false;
      const calls = stubBackend({
        install: () => {
          installed = true;
          return json({ ...IDLE, install: { phase: "downloading", downloaded: 4, total: 10 } }, 202);
        },
        status: () => json(installed ? { ...IDLE, install: { phase: "downloading", downloaded: 4, total: 10 } } : IDLE),
      });
      renderWithClient(<UpdateBanner />);

      expect(await screen.findByText("新しいバージョン 0.9.0 があります。")).toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: "更新する" }));
      const dialog = screen.getByRole("dialog", { name: "バージョン 0.9.0 に更新" });
      expect(dialog).toHaveTextContent("目の拡大図を直しました。");
      expect(dialog).toHaveTextContent("終わると自動で開き直します");

      fireEvent.click(within(dialog).getByRole("button", { name: "更新する" }));
      await waitFor(() => expect(calls.some(({ call }) => call === "POST /api/update/install")).toBe(true));
      expect(calls.find(({ call }) => call === "POST /api/update/install")?.body).toEqual({ version: "0.9.0" });
      expect(await screen.findByText("ダウンロード中… 40%")).toBeInTheDocument();
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
      // No closing the banner while the download runs.
      expect(screen.queryByRole("button", { name: "閉じる" })).not.toBeInTheDocument();
    });

    it("waits for a generating project before updating", async () => {
      stubBackend({ projects: [{ id: "p1", title: "", created_at: "", status: "running", version_count: 1 }] });
      renderWithClient(<UpdateBanner />);

      fireEvent.click(await screen.findByRole("button", { name: "更新する" }));
      const dialog = screen.getByRole("dialog");
      expect(
        await screen.findByText("生成中のプロジェクトがあります。終わってから更新してください。"),
      ).toBeInTheDocument();
      expect(within(dialog).getByRole("button", { name: "更新する" })).toBeDisabled();
    });

    it("stays closed for the rest of the launch once dismissed", async () => {
      stubBackend();
      const first = renderWithClient(<UpdateBanner />);
      fireEvent.click(await screen.findByRole("button", { name: "閉じる" }));
      expect(screen.queryByText("新しいバージョン 0.9.0 があります。")).not.toBeInTheDocument();
      first.unmount();

      renderWithClient(<UpdateBanner />);
      await waitFor(() => expect(screen.queryByRole("button", { name: "閉じる" })).not.toBeInTheDocument());
      expect(screen.queryByText("新しいバージョン 0.9.0 があります。")).not.toBeInTheDocument();
    });

    it("shows nothing when this is the newest version or the check failed", async () => {
      stubBackend({ check: () => json({ ...IDLE, update: null }) });
      const newest = renderWithClient(<UpdateBanner />);
      await waitFor(() => expect(newest.container).toBeEmptyDOMElement());
      newest.unmount();

      stubBackend({ check: () => json({ detail: "update_check_failed: 404 Not Found" }, 502) });
      const failed = renderWithClient(<UpdateBanner />);
      await waitFor(() => expect(failed.container).toBeEmptyDOMElement());
    });

    it("does not ask at startup when the user turned it off, nor in a plain browser", async () => {
      window.localStorage.setItem(CHECK_ON_LAUNCH_STORAGE_KEY, "0");
      const calls = stubBackend();
      renderWithClient(<UpdateBanner />);
      await new Promise((resolve) => setTimeout(resolve, 20));
      expect(calls.some(({ call }) => call === "POST /api/update/check")).toBe(false);
      cleanup();

      window.localStorage.clear();
      delete window.__MASKETTE_BACKEND__;
      renderWithClient(<UpdateBanner />);
      await new Promise((resolve) => setTimeout(resolve, 20));
      expect(calls.some(({ call }) => call === "POST /api/update/check")).toBe(false);
    });
  });

  describe("UpdateSection", () => {
    it("is left out where updates are not supported", async () => {
      const calls = stubBackend({ status: () => json({ ...IDLE, supported: false }) });
      const { container } = renderWithClient(<UpdateSection />);
      await waitFor(() => expect(calls.some(({ call }) => call === "GET /api/update")).toBe(true));
      expect(container).toBeEmptyDOMElement();
    });

    it("shows the version and checks on demand", async () => {
      window.localStorage.setItem(CHECK_ON_LAUNCH_STORAGE_KEY, "0");
      const calls = stubBackend({ check: () => json({ ...IDLE, update: null }) });
      renderWithClient(<UpdateSection />);

      expect(await screen.findByText("現在のバージョン：0.1.0")).toBeInTheDocument();
      expect(calls.some(({ call }) => call === "POST /api/update/check")).toBe(false);
      fireEvent.click(screen.getByRole("button", { name: "今すぐ確認" }));
      expect(await screen.findByText("最新のバージョンです。")).toBeInTheDocument();
    });

    it("says why a check failed, in the screen's language", async () => {
      stubBackend({ check: () => json({ detail: "update_check_failed: 404 Not Found" }, 502) });
      renderWithClient(<UpdateSection />);

      expect(await screen.findByRole("alert")).toHaveTextContent(
        "新しいバージョンを確認できませんでした。ネットワークにつながっているか確認してください。詳細：404 Not Found",
      );
    });

    it("remembers whether to check at startup", async () => {
      stubBackend();
      renderWithClient(<UpdateSection />);
      const toggle = await screen.findByRole("checkbox", { name: "起動時に新しいバージョンを確認する" });

      expect(toggle).toBeChecked();
      fireEvent.click(toggle);
      expect(window.localStorage.getItem(CHECK_ON_LAUNCH_STORAGE_KEY)).toBe("0");
    });
  });
});
