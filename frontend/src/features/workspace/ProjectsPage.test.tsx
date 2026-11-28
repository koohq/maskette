import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import i18n from "../../i18n";
import { ProjectsPage } from "./ProjectsPage";

const project = {
  id: "p1",
  title: "黑双马尾",
  created_at: "2026-10-08T00:00:00Z",
  status: "idle",
  thumbnail_url: null,
  version_count: 2,
};

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
}

function renderPage() {
  return render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <MemoryRouter>
        <ProjectsPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

async function openMenu(item: string) {
  fireEvent.click(await screen.findByRole("button", { name: "项目操作" }));
  fireEvent.click(screen.getByRole("button", { name: item }));
}

describe("ProjectsPage", () => {
  beforeAll(async () => {
    await i18n.changeLanguage("zh-CN");
  });

  afterEach(() => {
    cleanup();
    delete window.__MASKETTE_BACKEND__;
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("names the app in the header when opened in a plain browser", () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => json([project])),
    );
    const { container } = renderPage();

    const header = container.querySelector("header")!;
    expect(header.textContent).toContain("Maskette");
    expect(header.querySelector("img")).toBeNull();
  });

  it("leaves the app's icon and name to the OS title bar in the Maskette window", () => {
    window.__MASKETTE_BACKEND__ = { baseUrl: "http://127.0.0.1:4321", token: "test-token" };
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => json([project])),
    );
    const { container } = renderPage();

    const header = container.querySelector("header")!;
    expect(header.textContent).not.toContain("Maskette");
    expect(header.querySelector("img")).toBeNull();
    expect(within(header).getByRole("link", { name: "设置" })).toBeTruthy();
  });

  it("deletes a project through the in-app dialog without using window.confirm", async () => {
    let projects = [project];
    const fetchMock = vi.fn(async (_url: string, init?: RequestInit) => {
      if (init?.method === "DELETE") {
        projects = [];
        return json({ deleted: true });
      }
      return json(projects);
    });
    vi.stubGlobal("fetch", fetchMock);
    const nativeConfirm = vi.spyOn(window, "confirm");
    renderPage();

    await openMenu("删除项目");
    expect(screen.getByText("确定删除“黑双马尾”？项目里的所有版本都会被删除，无法撤销。")).toBeTruthy();
    fireEvent.click(screen.getAllByRole("button", { name: "删除项目" }).at(-1)!);

    await waitFor(() => expect(screen.queryByText("黑双马尾")).toBeNull());
    expect(
      fetchMock.mock.calls.some(([url, init]) => url === "/api/agent/conversations/p1" && init?.method === "DELETE"),
    ).toBe(true);
    expect(nativeConfirm).not.toHaveBeenCalled();
  });

  it("keeps the dialog open and explains why when the project is still running", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async (_url: string, init?: RequestInit) =>
        init?.method === "DELETE" ? json({ detail: "conversation_running" }, 409) : json([project]),
      ),
    );
    renderPage();

    await openMenu("删除项目");
    fireEvent.click(screen.getAllByRole("button", { name: "删除项目" }).at(-1)!);

    expect((await screen.findByRole("alert")).textContent).toBe("助手正在处理，请稍后再试。");
    expect(screen.getByRole("dialog")).toBeTruthy();
  });

  it("renames a project through the in-app dialog", async () => {
    const fetchMock = vi.fn(async (_url: string, init?: RequestInit) =>
      init?.method === "PATCH"
        ? json({
            id: "p1",
            title: "新名字",
            created_at: project.created_at,
            locale: "zh-CN",
            status: "idle",
            running: false,
            last_seq: 0,
            events: [],
            state: { references: [], images: [], current_front_id: null, approved_front_id: null },
          })
        : json([project]),
    );
    vi.stubGlobal("fetch", fetchMock);
    renderPage();

    await openMenu("重命名");
    const input = screen.getByRole("textbox", { name: "重命名" });
    expect((input as HTMLInputElement).value).toBe("黑双马尾");
    fireEvent.change(input, { target: { value: " 新名字 " } });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    const patch = fetchMock.mock.calls.find(([, init]) => init?.method === "PATCH");
    expect(JSON.parse(String(patch?.[1]?.body))).toEqual({ title: "新名字" });
  });

  it("opens the new-project dialog with its field focused and closes it on Escape", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => json([project])),
    );
    renderPage();

    const opener = screen.getAllByRole("button", { name: "新建项目" })[0]!;
    opener.focus();
    fireEvent.click(opener);
    const dialog = screen.getByRole("dialog", { name: "新建项目" });
    expect(screen.getByRole("textbox", { name: "新建项目" })).toHaveFocus();
    expect(screen.getByRole("button", { name: "创建" })).toBeEnabled();

    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(opener).toHaveFocus();
  });

  it("creates a project without a name and shows a failure inside the dialog", async () => {
    const fetchMock = vi.fn(async (_url: string, init?: RequestInit) =>
      init?.method === "POST" ? json({ detail: "conversation_running" }, 409) : json([project]),
    );
    vi.stubGlobal("fetch", fetchMock);
    renderPage();

    fireEvent.click(screen.getAllByRole("button", { name: "新建项目" })[0]!);
    fireEvent.click(screen.getByRole("button", { name: "创建" }));

    expect((await screen.findByRole("alert")).textContent).toBe("助手正在处理，请稍后再试。");
    expect(screen.getByRole("dialog", { name: "新建项目" })).toBeTruthy();
    expect(fetchMock.mock.calls.some(([, init]) => init?.method === "POST")).toBe(true);
  });

  it("offers the .maskette export in the project menu and an import button", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => json([project])),
    );
    renderPage();

    expect(screen.getByRole("button", { name: "导入项目" })).toBeTruthy();
    fireEvent.click(await screen.findByRole("button", { name: "项目操作" }));
    const exportLink = await screen.findByRole("link", { name: "导出 .maskette" });
    expect(exportLink.getAttribute("href")).toBe("/api/agent/conversations/p1/export");
    expect(exportLink.getAttribute("download")).toBe("");
  });

  it("imports a project file and opens the restored project", async () => {
    const restored = {
      id: "p2",
      title: "导入的工程",
      created_at: project.created_at,
      locale: "zh-CN",
      status: "idle",
      running: false,
      last_seq: 0,
      events: [],
      state: { references: [], images: [], current_front_id: null, approved_front_id: null },
    };
    const fetchMock = vi.fn(async (url: string, init?: RequestInit) =>
      init?.method === "POST" && url === "/api/agent/conversations/import" ? json(restored) : json([project]),
    );
    vi.stubGlobal("fetch", fetchMock);
    const { container } = renderPage();

    fireEvent.change(container.querySelector('input[type="file"][accept=".maskette,.vkp"]')!, {
      target: { files: [new File(["bundle"], "project.maskette", { type: "application/zip" })] },
    });

    await waitFor(() =>
      expect(fetchMock.mock.calls.some(([url]) => url === "/api/agent/conversations/import")).toBe(true),
    );
  });

  it("says the list failed to load instead of showing an empty page, and retries", async () => {
    let up = false;
    const fetchMock = vi.fn(async () => {
      if (!up) throw new TypeError("Failed to fetch");
      return json([project]);
    });
    vi.stubGlobal("fetch", fetchMock);
    renderPage();

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("无法加载项目列表。");
    expect(screen.queryByText("还没有项目。新建一个，上传角色参考图开始设计。")).toBeNull();

    up = true;
    fireEvent.click(screen.getByRole("button", { name: "重试" }));
    expect(await screen.findByText(project.title)).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
