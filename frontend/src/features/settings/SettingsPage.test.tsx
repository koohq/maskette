import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import i18n from "../../i18n";
import { SettingsPage } from "./SettingsPage";
import type { BackendSettings, ModelCandidate } from "./settingsApi";

const initial: BackendSettings = {
  values: {
    agent_llm_provider: "openai_compatible",
    agent_llm_base_url: "https://example.test/v1",
    agent_llm_model: "chat-model",
    agent_llm_extra_body: '{"thinking": {"type": "disabled"}}',
    agent_codex_model: "",
    agent_claude_code_model: "",
    llm_provider: "codex",
    analysis_llm_base_url: "",
    analysis_llm_model: "",
    analysis_llm_extra_body: "",
    image_provider: "codex",
    siliconflow_image_model: "sf-model",
    ark_image_model: "ark-model",
    openrouter_base_url: "https://openrouter.ai/api/v1",
    openrouter_image_model: "",
    openrouter_extra_body: "",
  },
  overridden: [],
  options: {
    agent_llm_provider: ["openai_compatible", "codex", "claude_code"],
    llm_provider: ["same_as_agent", "openai_compatible", "codex", "claude_code"],
    image_provider: ["openrouter", "codex", "siliconflow", "ark"],
  },
  keys: { "api.siliconflow.cn": { hint: "…0000" } },
  status: {
    agent_llm_key: true,
    analysis_llm_key: false,
    claude_cli: true,
    codex_cli: true,
    siliconflow_key: true,
    ark_key: false,
    openrouter_key: true,
  },
};

/** Answers the settings API: the model lists with `models` (none by default), everything else with `settings`. */
function backend(settings: BackendSettings, models: (url: string) => ModelCandidate[] = () => []) {
  return vi.fn(async (url: string, _init?: RequestInit) =>
    url.startsWith("/api/settings/models/")
      ? new Response(JSON.stringify({ models: models(url) }))
      : new Response(JSON.stringify(settings)),
  );
}

function renderPage() {
  return render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <MemoryRouter>
        <SettingsPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("SettingsPage", () => {
  beforeAll(async () => {
    await i18n.changeLanguage("zh-CN");
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  it("saves only the changed backend values", async () => {
    const fetchMock = vi.fn(async (_url: string, init?: RequestInit) => {
      if (init?.method === "PUT") {
        const saved = {
          ...initial,
          values: { ...initial.values, image_provider: "siliconflow" },
          overridden: ["image_provider"],
        };
        return new Response(JSON.stringify(saved));
      }
      return new Response(JSON.stringify(initial));
    });
    vi.stubGlobal("fetch", fetchMock);
    renderPage();

    const siliconflow = await screen.findByRole("radio", { name: "硅基流动 SiliconFlow" });
    expect(screen.getByRole("button", { name: "保存" })).toHaveProperty("disabled", true);
    fireEvent.click(siliconflow);
    expect(screen.getByDisplayValue("sf-model")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await screen.findByText("已保存，下一次请求起生效");
    const put = fetchMock.mock.calls.find(([, init]) => init?.method === "PUT");
    expect(JSON.parse(String(put?.[1]?.body))).toEqual({ values: { image_provider: "siliconflow" } });
    expect(screen.getByText("已修改")).toBeTruthy();
  });

  it("shows the OpenRouter image fields and saves its model", async () => {
    const fetchMock = vi.fn(async (_url: string, init?: RequestInit) => {
      if (init?.method === "PUT") {
        const saved = {
          ...initial,
          values: { ...initial.values, image_provider: "openrouter", openrouter_image_model: "vendor/image-model" },
          overridden: ["image_provider", "openrouter_image_model"],
        };
        return new Response(JSON.stringify(saved));
      }
      return new Response(JSON.stringify(initial));
    });
    vi.stubGlobal("fetch", fetchMock);
    renderPage();

    fireEvent.click(await screen.findByRole("radio", { name: "OpenRouter" }));
    expect(screen.getByDisplayValue("https://openrouter.ai/api/v1")).toBeTruthy();
    // The model field is the one whose hint points at the image model list.
    const modelInput = screen.getByText(/images\/models/).parentElement?.querySelector("input");
    expect(modelInput).toBeTruthy();
    fireEvent.change(modelInput as HTMLInputElement, { target: { value: "vendor/image-model" } });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await screen.findByText("已保存，下一次请求起生效");
    const put = fetchMock.mock.calls.find(([, init]) => init?.method === "PUT");
    expect(JSON.parse(String(put?.[1]?.body))).toEqual({
      values: { image_provider: "openrouter", openrouter_image_model: "vendor/image-model" },
    });
  });

  it("sends a typed API key once for its service and never shows the saved one", async () => {
    const ark = {
      ...initial,
      values: { ...initial.values, ark_base_url: "https://ark.cn-beijing.volces.com/api/plan/v3" },
    };
    const fetchMock = vi.fn(async (_url: string, init?: RequestInit) => {
      if (init?.method === "PUT") {
        const saved = {
          ...ark,
          values: { ...ark.values, image_provider: "ark" },
          keys: { ...ark.keys, "ark.cn-beijing.volces.com": { hint: "…9999" } },
          overridden: ["image_provider"],
          status: { ...ark.status, ark_key: true },
        };
        return new Response(JSON.stringify(saved));
      }
      return new Response(JSON.stringify(ark));
    });
    vi.stubGlobal("fetch", fetchMock);
    renderPage();

    fireEvent.click(await screen.findByRole("radio", { name: "火山方舟" }));
    expect(screen.getByText("需要填写方舟 API Key")).toBeTruthy();
    expect(screen.getByText("火山方舟（中国大陆版）的 API Key · 未配置")).toBeTruthy();
    fireEvent.change(screen.getAllByPlaceholderText("粘贴 API Key").at(-1)!, {
      target: { value: " ark-new-key-9999 " },
    });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await screen.findByText("火山方舟（中国大陆版）的 API Key · 已配置 …9999");
    const put = fetchMock.mock.calls.find(([, init]) => init?.method === "PUT");
    expect(JSON.parse(String(put?.[1]?.body))).toEqual({
      values: { image_provider: "ark" },
      keys: { "ark.cn-beijing.volces.com": "ark-new-key-9999" },
    });
    expect((screen.getByPlaceholderText("留空保持不变，输入新 Key 则替换") as HTMLInputElement).value).toBe("");

    fireEvent.click(screen.getByRole("button", { name: "清除此 Key" }));
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(fetchMock.mock.calls.filter(([, init]) => init?.method === "PUT")).toHaveLength(2));
    const clear = fetchMock.mock.calls.filter(([, init]) => init?.method === "PUT")[1]!;
    expect(JSON.parse(String(clear[1]?.body))).toEqual({ values: {}, keys: { "ark.cn-beijing.volces.com": null } });
  });

  it("shares one key among the sections that use one service", async () => {
    const shared = {
      ...initial,
      values: {
        ...initial.values,
        agent_llm_base_url: "https://openrouter.ai/api/v1",
        llm_provider: "same_as_agent",
        image_provider: "openrouter",
      },
      keys: { "openrouter.ai": { hint: "…1234" } },
    };
    const fetchMock = backend(shared);
    vi.stubGlobal("fetch", fetchMock);
    renderPage();

    const chat = (await screen.findByRole("radiogroup", { name: "对话助手 LLM" })).closest("section") as HTMLElement;
    const image = screen.getByRole("radiogroup", { name: "生图后端" }).closest("section") as HTMLElement;
    expect(within(chat).getByText("OpenRouter 的 API Key · 已配置 …1234")).toBeTruthy();
    expect(within(chat).getByText("此 Key 也用于“参考图分析 LLM”和“生图后端”。")).toBeTruthy();
    expect(within(image).getByText("OpenRouter 的 API Key · 已配置 …1234")).toBeTruthy();
    expect(within(image).getByText("此 Key 也用于“对话助手 LLM”和“参考图分析 LLM”。")).toBeTruthy();

    // Typed in one section, the key shows in the other's field too.
    const chatKey = within(chat).getByPlaceholderText("留空保持不变，输入新 Key 则替换") as HTMLInputElement;
    const imageKey = within(image).getByPlaceholderText("留空保持不变，输入新 Key 则替换") as HTMLInputElement;
    fireEvent.change(chatKey, { target: { value: "sk-or-new-0000" } });
    expect(imageKey.value).toBe("sk-or-new-0000");

    // On another service the chat (and the analysis that follows it) has a key of its own.
    fireEvent.click(within(chat).getByRole("button", { name: "OpenAI" }));
    expect(within(chat).getByText("OpenAI 的 API Key · 未配置")).toBeTruthy();
    expect(within(chat).getByText("此 Key 也用于“参考图分析 LLM”。")).toBeTruthy();
    expect(within(image).queryByText(/此 Key 也用于/)).toBeNull();
    expect(imageKey.value).toBe("sk-or-new-0000");
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(fetchMock.mock.calls.some(([, init]) => init?.method === "PUT")).toBe(true));
    const put = fetchMock.mock.calls.find(([, init]) => init?.method === "PUT");
    expect(JSON.parse(String(put?.[1]?.body))).toEqual({
      values: { agent_llm_base_url: "https://api.openai.com/v1" },
      keys: { "openrouter.ai": "sk-or-new-0000" },
    });
  });

  it("saves no key typed for a URL that changed since", async () => {
    const fetchMock = backend(initial);
    vi.stubGlobal("fetch", fetchMock);
    renderPage();

    const chat = (await screen.findByRole("radiogroup", { name: "对话助手 LLM" })).closest("section") as HTMLElement;
    fireEvent.change(within(chat).getByPlaceholderText("粘贴 API Key"), { target: { value: "sk-for-example" } });
    fireEvent.click(within(chat).getByRole("button", { name: "OpenAI" }));
    // The field shows OpenAI's key now, which is not set.
    expect((within(chat).getByPlaceholderText("粘贴 API Key") as HTMLInputElement).value).toBe("");
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(fetchMock.mock.calls.some(([, init]) => init?.method === "PUT")).toBe(true));
    const put = fetchMock.mock.calls.find(([, init]) => init?.method === "PUT");
    expect(JSON.parse(String(put?.[1]?.body))).toEqual({ values: { agent_llm_base_url: "https://api.openai.com/v1" } });
  });

  it("lets the analysis LLM use an OpenAI-compatible API that defaults to the chat assistant", async () => {
    const fetchMock = backend(initial);
    vi.stubGlobal("fetch", fetchMock);
    renderPage();

    const analysis = await screen.findByRole("radiogroup", { name: "参考图分析 LLM" });
    const radio = analysis.querySelector<HTMLInputElement>('input[value="openai_compatible"]')!;
    fireEvent.click(radio);

    expect(screen.getByPlaceholderText("留空沿用对话助手：https://example.test/v1")).toBeTruthy();
    expect(screen.getByPlaceholderText("留空沿用对话助手：chat-model")).toBeTruthy();
    expect(screen.getByText("需要填写 API Key")).toBeTruthy();
    // With no URL of its own, analysis uses the chat assistant's, and so its key.
    expect(screen.getByText("此 Key 也用于“参考图分析 LLM”。")).toBeTruthy();
    expect(screen.getByText("此 Key 也用于“对话助手 LLM”。")).toBeTruthy();
    expect(screen.getByText("模型必须支持图片输入（多模态），否则分析会失败。")).toBeTruthy();
  });

  it("reuses the chat assistant's extra fields only while analysis shares its URL", async () => {
    vi.stubGlobal("fetch", backend(initial));
    renderPage();

    const analysis = await screen.findByRole("radiogroup", { name: "参考图分析 LLM" });
    fireEvent.click(analysis.querySelector<HTMLInputElement>('input[value="openai_compatible"]')!);
    const sameAsAgent = '留空沿用对话助手：{"thinking": {"type": "disabled"}}';
    const extraBody = screen.getByPlaceholderText(sameAsAgent);

    fireEvent.change(screen.getByPlaceholderText("留空沿用对话助手：https://example.test/v1"), {
      target: { value: "https://router.test/api/v1" },
    });
    expect(screen.queryByPlaceholderText(sameAsAgent)).toBeNull();
    expect(extraBody.getAttribute("placeholder")).toBe("{}");
  });

  it("tests the saved chat assistant and explains each check", async () => {
    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      if (url === "/api/settings/probe/agent" && init?.method === "POST") {
        return new Response(
          JSON.stringify({
            ok: true,
            model: "glm-5",
            base_url: "https://example.test/v1",
            latency_ms: 812,
            checks: [
              { id: "connection", status: "ok", detail: "" },
              { id: "extra_body", status: "warn", detail: "thinking" },
              { id: "tools", status: "ok", detail: "" },
            ],
          }),
        );
      }
      return new Response(JSON.stringify(initial));
    });
    vi.stubGlobal("fetch", fetchMock);
    renderPage();

    const button = await screen.findByRole("button", { name: "测试连接" });
    fireEvent.change(screen.getByDisplayValue("chat-model"), { target: { value: "other" } });
    expect(button).toHaveProperty("disabled", true);
    expect(screen.getByText("有未保存的修改，先保存再测试。")).toBeTruthy();
    fireEvent.change(screen.getByDisplayValue("other"), { target: { value: "chat-model" } });
    fireEvent.click(button);

    await screen.findByText("glm-5 可用 · 812 ms");
    expect(screen.getByText("模型不支持额外参数 thinking，已自动去掉。建议清空“额外请求参数”。")).toBeTruthy();
    expect(screen.getByText("支持工具调用")).toBeTruthy();
  });

  it("explains a failed connection", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) =>
        url === "/api/settings/probe/agent"
          ? new Response(
              JSON.stringify({
                ok: false,
                model: "glm-5",
                base_url: "x",
                latency_ms: null,
                checks: [{ id: "connection", status: "fail", detail: "auth: HTTP 401: bad key" }],
              }),
            )
          : new Response(JSON.stringify(initial)),
      ),
    );
    renderPage();

    fireEvent.click(await screen.findByRole("button", { name: "测试连接" }));

    await screen.findByText("glm-5 不可用");
    expect(screen.getByText("Key 无效或没有权限（HTTP 401: bad key）")).toBeTruthy();
  });

  it("recommends the API-key backends and notes the subscription CLIs are for personal use", async () => {
    vi.stubGlobal("fetch", backend(initial));
    renderPage();

    await screen.findAllByText("推荐");
    expect(screen.getAllByText("推荐")).toHaveLength(3);
    // Analysis and image generation both start on Codex; the chat assistant uses an API key.
    expect(screen.getAllByRole("note")).toHaveLength(2);
    expect(screen.getAllByRole("note")[0]?.textContent).toContain("个人 ChatGPT 或 Claude 订阅");

    fireEvent.click(screen.getByRole("radio", { name: "OpenRouter" }));
    expect(screen.getAllByRole("note")).toHaveLength(1);
    fireEvent.click(screen.getAllByRole("radio", { name: "Claude Code CLI" })[0] as HTMLElement);
    expect(screen.getAllByRole("note")).toHaveLength(2);
  });

  it("runs the chat assistant on Codex with only a model field", async () => {
    const fetchMock = vi.fn(async (_url: string, init?: RequestInit) => {
      if (init?.method === "PUT") {
        const saved = {
          ...initial,
          values: { ...initial.values, agent_llm_provider: "codex", agent_codex_model: "codex-model" },
          overridden: ["agent_codex_model", "agent_llm_provider"],
        };
        return new Response(JSON.stringify(saved));
      }
      return new Response(JSON.stringify(initial));
    });
    vi.stubGlobal("fetch", fetchMock);
    renderPage();

    const chat = await screen.findByRole("radiogroup", { name: "对话助手 LLM" });
    fireEvent.click(chat.querySelector<HTMLInputElement>('input[value="codex"]')!);
    // No API URL, key or extra fields: Codex runs on the user's own login.
    expect(screen.queryByDisplayValue("https://example.test/v1")).toBeNull();
    expect(screen.queryByDisplayValue("chat-model")).toBeNull();
    // Analysis, image generation and now the chat all drive a subscription CLI.
    expect(screen.getAllByRole("note")).toHaveLength(3);
    fireEvent.change(screen.getByPlaceholderText("留空使用默认模型"), { target: { value: "codex-model" } });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await screen.findByText("已保存，下一次请求起生效");
    const put = fetchMock.mock.calls.find(([, init]) => init?.method === "PUT");
    expect(JSON.parse(String(put?.[1]?.body))).toEqual({
      values: { agent_llm_provider: "codex", agent_codex_model: "codex-model" },
    });
  });

  it("says a saved Codex Bridge is gone and offers no Bridge fields", async () => {
    const saved = { ...initial, values: { ...initial.values, image_provider: "codex_bridge" } };
    vi.stubGlobal("fetch", backend(saved));
    renderPage();

    expect((await screen.findByRole("alert")).textContent).toBe("Codex Bridge 已不再提供，请选择 Codex 或其他后端。");
    expect(screen.queryByText(/Bridge 地址/)).toBeNull();
    fireEvent.click(screen.getAllByRole("radio", { name: "Codex CLI" })[2] as HTMLElement);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("warns when the selected backend is not configured", async () => {
    const unconfigured = { ...initial, values: { ...initial.values, image_provider: "ark" } };
    vi.stubGlobal("fetch", backend(unconfigured));
    renderPage();

    expect(await screen.findByText("需要填写方舟 API Key")).toBeTruthy();
  });

  it("describes every choice in one line", async () => {
    vi.stubGlobal("fetch", backend(initial));
    renderPage();

    await screen.findByRole("radiogroup", { name: "生图后端" });
    const radios = ["对话助手 LLM", "参考图分析 LLM", "生图后端"].flatMap((name) =>
      within(screen.getByRole("radiogroup", { name })).getAllByRole("radio"),
    );
    // Chat 3, analysis 3 (unfolded: it is on Codex), images 4.
    expect(radios).toHaveLength(10);
    for (const radio of radios) {
      const note = document.getElementById(radio.getAttribute("aria-describedby") ?? "");
      expect(note?.textContent, radio.getAttribute("value") ?? "").toBeTruthy();
    }
    expect(screen.getByText("通过 API Key 使用硅基流动的图像模型。")).toBeTruthy();
    expect(screen.getByText("用本机的 Codex CLI 生成图片，以 ChatGPT 付费订阅登录。")).toBeTruthy();
    // The image backends list the recommended one first, as the other roles do.
    const images = screen.getByRole("radiogroup", { name: "生图后端" });
    expect([...images.querySelectorAll("input")].map((input) => input.value)).toEqual([
      "openrouter",
      "codex",
      "siliconflow",
      "ark",
    ]);
  });

  it("offers the test fixtures of a debug build, with a note of the chat's own", async () => {
    const debug = {
      ...initial,
      options: Object.fromEntries(
        Object.entries(initial.options).map(([key, options]) => [key, [...options, "fixture"]]),
      ),
    };
    vi.stubGlobal("fetch", backend(debug));
    renderPage();

    const chat = await screen.findByRole("radiogroup", { name: "对话助手 LLM" });
    const note = (name: string) => {
      const radio = within(screen.getByRole("radiogroup", { name })).getByDisplayValue("fixture");
      return document.getElementById(radio.getAttribute("aria-describedby") ?? "")?.textContent;
    };
    expect(note("对话助手 LLM")).toBe(
      "开发用：不调用 AI，每次发送都按固定步骤进入下一步。分析和图像生成仍按各自的设置运行。",
    );
    expect(note("参考图分析 LLM")).toBe("开发用：不调用 AI，返回固定的示例结果。");
    expect(note("生图后端")).toBe("开发用：不调用 AI，返回固定的示例图片。");
    // Nothing to set up: no API URL or model.
    fireEvent.click(within(chat).getByDisplayValue("fixture"));
    expect(screen.queryByDisplayValue("https://example.test/v1")).toBeNull();
    expect(screen.queryByDisplayValue("chat-model")).toBeNull();
  });

  it("folds analysis onto the chat assistant's settings and unfolds it to a backend of its own", async () => {
    const following = { ...initial, values: { ...initial.values, llm_provider: "same_as_agent" } };
    const fetchMock = backend(following);
    vi.stubGlobal("fetch", fetchMock);
    renderPage();

    const same = await screen.findByRole("checkbox", { name: "分析也使用对话助手的设置" });
    expect(same).toHaveProperty("checked", true);
    expect(screen.queryByRole("radiogroup", { name: "参考图分析 LLM" })).toBeNull();
    expect(
      screen.getByText("分析使用与对话助手相同的“OpenAI 兼容 API”中的 chat-model。该模型需要支持图片输入。"),
    ).toBeTruthy();
    // The shared model can be tested for image input without unfolding.
    expect(screen.getByTestId("probe-analysis")).toBeTruthy();

    fireEvent.click(screen.getAllByRole("radio", { name: "Claude Code CLI" })[0] as HTMLElement);
    expect(screen.getByText("分析使用与对话助手相同的“Claude Code CLI”。")).toBeTruthy();
    expect(screen.queryByTestId("probe-analysis")).toBeNull();

    // Unfolded, analysis keeps the backend it was using.
    fireEvent.click(same);
    const analysis = screen.getByRole("radiogroup", { name: "参考图分析 LLM" });
    expect(analysis.querySelector<HTMLInputElement>('input[value="claude_code"]')?.checked).toBe(true);
    expect(analysis.querySelector('input[value="same_as_agent"]')).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(fetchMock.mock.calls.some(([, init]) => init?.method === "PUT")).toBe(true));
    const put = fetchMock.mock.calls.find(([, init]) => init?.method === "PUT");
    expect(JSON.parse(String(put?.[1]?.body))).toEqual({
      values: { agent_llm_provider: "claude_code", llm_provider: "claude_code" },
    });
  });

  it("shows analysis settings of its own unfolded, and folds them back", async () => {
    const fetchMock = backend(initial);
    vi.stubGlobal("fetch", fetchMock);
    renderPage();

    const same = await screen.findByRole("checkbox", { name: "分析也使用对话助手的设置" });
    expect(same).toHaveProperty("checked", false);
    fireEvent.click(same);
    expect(screen.queryByRole("radiogroup", { name: "参考图分析 LLM" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(fetchMock.mock.calls.some(([, init]) => init?.method === "PUT")).toBe(true));
    const put = fetchMock.mock.calls.find(([, init]) => init?.method === "PUT");
    expect(JSON.parse(String(put?.[1]?.body))).toEqual({ values: { llm_provider: "same_as_agent" } });
  });

  it("fills the API URL from a preset", async () => {
    vi.stubGlobal("fetch", backend(initial));
    renderPage();

    const chat = (await screen.findByRole("radiogroup", { name: "对话助手 LLM" })).closest("section") as HTMLElement;
    const presets = within(chat).getByRole("group", { name: "预设" });
    fireEvent.click(within(presets).getByRole("button", { name: "Google Gemini" }));

    expect(within(chat).getByDisplayValue("https://generativelanguage.googleapis.com/v1beta/openai")).toBeTruthy();
    expect(within(presets).getByRole("button", { name: "Google Gemini" }).getAttribute("aria-pressed")).toBe("true");
    expect(within(chat).getByText("Google Gemini 的模型名不要加 models/。")).toBeTruthy();
    // Volcengine Ark's is the URL of the Ark image settings, so the chat can share the Ark key.
    fireEvent.click(within(presets).getByRole("button", { name: "火山方舟" }));
    expect(within(chat).getByDisplayValue("https://ark.cn-beijing.volces.com/api/plan/v3")).toBeTruthy();
  });

  it("offers the international site of a service that has one", async () => {
    vi.stubGlobal("fetch", backend(initial));
    renderPage();

    const chat = (await screen.findByRole("radiogroup", { name: "对话助手 LLM" })).closest("section") as HTMLElement;
    const presets = within(chat).getByRole("group", { name: "预设" });
    // A service with one site has no region row.
    fireEvent.click(within(presets).getByRole("button", { name: "OpenAI" }));
    expect(within(chat).queryByRole("group", { name: "地区" })).toBeNull();

    fireEvent.click(within(presets).getByRole("button", { name: "火山方舟" }));
    const regions = within(chat).getByRole("group", { name: "地区" });
    expect(within(regions).getByRole("button", { name: "中国大陆版" }).getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(within(regions).getByRole("button", { name: "国际版" }));
    expect(within(chat).getByDisplayValue("https://ark.ap-southeast.bytepluses.com/api/v3")).toBeTruthy();
    expect(within(presets).getByRole("button", { name: "火山方舟" }).getAttribute("aria-pressed")).toBe("true");
    expect(within(chat).getByText(/^国际版 BytePlus ModelArk 的地址。/)).toBeTruthy();

    // Moving to the other two-site service keeps the international site.
    fireEvent.click(within(presets).getByRole("button", { name: "硅基流动" }));
    expect(within(chat).getByDisplayValue("https://api.siliconflow.com/v1")).toBeTruthy();
    expect(within(chat).getByText(/^国际版的地址。/)).toBeTruthy();
  });

  it("offers only the image service's own sites on its URL", async () => {
    const fetchMock = backend({ ...initial, values: { ...initial.values, image_provider: "ark" } });
    vi.stubGlobal("fetch", fetchMock);
    renderPage();

    const image = (await screen.findByRole("radiogroup", { name: "生图后端" })).closest("section") as HTMLElement;
    expect(within(image).queryByRole("group", { name: "预设" })).toBeNull();
    const regions = within(image).getByRole("group", { name: "地区" });
    // An empty or own URL matches neither site.
    for (const button of within(regions).getAllByRole("button")) {
      expect(button.getAttribute("aria-pressed")).toBe("false");
    }
    fireEvent.click(within(regions).getByRole("button", { name: "国际版" }));
    expect(within(image).getByDisplayValue("https://ark.ap-southeast.bytepluses.com/api/v3")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(fetchMock.mock.calls.some(([, init]) => init?.method === "PUT")).toBe(true));
    const put = fetchMock.mock.calls.find(([, init]) => init?.method === "PUT");
    expect(JSON.parse(String(put?.[1]?.body))).toEqual({
      values: { ark_base_url: "https://ark.ap-southeast.bytepluses.com/api/v3" },
    });
  });

  it("offers OpenRouter's models with their prices and still takes a typed name", async () => {
    const fetchMock = backend(initial, (url) =>
      url.startsWith("/api/settings/models/agent?") && url.includes(encodeURIComponent("https://openrouter.ai/api/v1"))
        ? [
            {
              id: "google/gemini-3.8-flash",
              name: "Google: Gemini 3.8 Flash",
              price: { unit: "token", input: 0.3, output: 2.5 },
            },
            { id: "vendor/free-model", name: "", price: { unit: "token", input: 0, output: 0 } },
          ]
        : [],
    );
    vi.stubGlobal("fetch", fetchMock);
    renderPage();

    const model = await screen.findByDisplayValue("chat-model");
    // Not OpenRouter: a plain field.
    expect(model.getAttribute("role")).toBeNull();
    const chat = screen.getByRole("radiogroup", { name: "对话助手 LLM" }).closest("section") as HTMLElement;
    fireEvent.click(within(chat).getByRole("button", { name: "OpenRouter" }));

    const combobox = await within(chat).findByRole("combobox");
    fireEvent.click(within(chat).getByRole("button", { name: "展开模型列表" }));
    const list = within(chat).getByRole("listbox");
    expect(within(list).getByRole("option", { name: /^google\/gemini-3\.8-flash ?\$0\.3 \/ \$2\.5$/ })).toBeTruthy();
    expect(within(list).getByRole("option", { name: /^vendor\/free-model ?免费$/ })).toBeTruthy();
    expect(within(chat).getByText(/支持工具调用的模型/)).toBeTruthy();
    fireEvent.click(within(list).getByRole("option", { name: /gemini/ }));
    expect((combobox as HTMLInputElement).value).toBe("google/gemini-3.8-flash");
    expect(within(chat).queryByRole("listbox")).toBeNull();

    // Typing filters the list, and a name it does not have stays as typed.
    fireEvent.change(combobox, { target: { value: "FREE" } });
    expect(
      within(chat)
        .getAllByRole("option")
        .map((option) => option.textContent),
    ).toEqual(["vendor/free-model免费"]);
    fireEvent.change(combobox, { target: { value: "vendor/unlisted" } });
    expect(within(chat).getByText("没有匹配的模型。输入的模型名也可以直接使用。")).toBeTruthy();
    fireEvent.keyDown(combobox, { key: "Escape" });
    expect((combobox as HTMLInputElement).value).toBe("vendor/unlisted");
  });
});
