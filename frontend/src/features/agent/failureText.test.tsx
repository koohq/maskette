import { cleanup, render, screen } from "@testing-library/react";
import { afterAll, afterEach, describe, expect, it, vi } from "vitest";
import i18n from "../../i18n";
import type { AgentEvent, Conversation } from "./agentApi";
import { ChatTimeline } from "./ChatTimeline";
import { FAILURE_CODES, failureText } from "./failureText";

function conversation(): Conversation {
  return {
    id: "c",
    title: "",
    created_at: "",
    locale: "ja",
    status: "error",
    running: false,
    last_seq: 0,
    events: [],
    state: {
      references: [],
      missing_views: [],
      images: [],
      current_design_id: null,
      approved_design_id: null,
      current_front_id: null,
      approved_front_id: null,
    },
  };
}

// What the backend sends when the CLIs are missing (agent/runner.rs).
const failedTurn: AgentEvent[] = [
  { seq: 1, type: "user_message", text: "正面図を作って" },
  {
    seq: 2,
    type: "tool",
    tool: "generate_front_view",
    status: "failed",
    error: "Generation failed: The Codex CLI (codex) was not found. Install it and log in.",
    error_code: "cli_not_found",
    error_params: { name: "Codex", program: "codex" },
  },
  {
    seq: 3,
    type: "error",
    message: "Claude Code has reached its usage limit.",
    code: "cli_usage_limit",
    params: { name: "Claude Code" },
  },
];

function renderTimeline(events: AgentEvent[]) {
  render(
    <ChatTimeline
      conversation={conversation()}
      events={events}
      onApprove={vi.fn()}
      onOpenImage={vi.fn()}
      running={false}
    />,
  );
}

afterEach(cleanup);

afterAll(async () => {
  await i18n.changeLanguage("zh-CN");
});

describe("CLI failures", () => {
  it("are shown in Japanese from their code", async () => {
    await i18n.changeLanguage("ja");
    renderTimeline(failedTurn);
    expect(
      screen.getByText(
        "Codex の CLI（codex）が見つかりません。インストールしてログインするか、設定で API キー方式に切り替えてください。",
      ),
    ).toBeTruthy();
    expect(
      screen.getByText(
        "エラーが発生しました：Claude Code の利用上限に達しました。時間をおいて試すか、設定で API キー方式に切り替えてください。",
      ),
    ).toBeTruthy();
    expect(screen.queryByText(/was not found/)).toBeNull();
  });

  it("are shown in English from their code", async () => {
    await i18n.changeLanguage("en");
    renderTimeline(failedTurn);
    expect(
      screen.getByText(
        "The Codex CLI (codex) was not found. Install it and log in, or choose a backend that uses an API key in the settings.",
      ),
    ).toBeTruthy();
    expect(screen.getByText(/Claude Code has reached its usage limit\. Try again later/)).toBeTruthy();
  });

  it("add the exit code, the outdated hint and the CLI's own words", async () => {
    await i18n.changeLanguage("ja");
    const params = { name: "Claude Code", exit_code: 1, outdated: true, detail: "error: unknown option" };
    expect(failureText(i18n.t, "cli_failed", params, "fallback")).toBe(
      "Claude Code の CLI が失敗しました。終了コードは 1 でした。CLI が古い可能性があります。更新してから試してください。詳細：error: unknown option",
    );
    await i18n.changeLanguage("en");
    expect(failureText(i18n.t, "cli_failed", params, "fallback")).toBe(
      "The Claude Code CLI failed. Exit code 1. The CLI may be out of date. Update it and try again. Details: error: unknown option",
    );
  });

  it("without a known code keep the backend's text", async () => {
    await i18n.changeLanguage("ja");
    expect(failureText(i18n.t, undefined, undefined, "Agent LLM HTTP 500: boom")).toBe("Agent LLM HTTP 500: boom");
    expect(failureText(i18n.t, "something_new", {}, "as sent")).toBe("as sent");
  });

  it("have a translation for every code in each language", async () => {
    for (const language of ["ja", "en", "zh-CN"]) {
      await i18n.changeLanguage(language);
      for (const code of FAILURE_CODES) {
        const key = `agent.failures.${code}`;
        expect(i18n.exists(key, { lng: language, fallbackLng: false }), `${language} ${key}`).toBe(true);
      }
    }
  });
});

// What the backend sends for a hosted API, missing settings, a stop and a refused reference.
const apiTurn: AgentEvent[] = [
  { seq: 1, type: "user_message", text: "正面図を作って" },
  {
    seq: 2,
    type: "tool",
    tool: "generate_front_view",
    status: "failed",
    error: "Generation failed: OpenRouter image generation failed: HTTP 401 trace_id=t: No auth credentials found",
    error_code: "api_unauthorized",
    error_params: { service: "OpenRouter", status: 401, detail: "No auth credentials found" },
  },
  {
    seq: 3,
    type: "error",
    message: "Agent LLM stream failed: no answer within 180s",
    code: "api_timeout",
    params: { service: "openrouter.ai", seconds: 180 },
  },
];

describe("API and other failures", () => {
  it("are shown in Japanese from their code", async () => {
    await i18n.changeLanguage("ja");
    renderTimeline(apiTurn);
    expect(
      screen.getByText(
        "OpenRouter が API キーを受け付けませんでした（HTTP 401）。設定の API キーを確認してください。詳細：No auth credentials found",
      ),
    ).toBeTruthy();
    expect(
      screen.getByText(
        "エラーが発生しました：openrouter.ai が 180 秒以内に応答しませんでした。もう一度試してください。",
      ),
    ).toBeTruthy();
    expect(screen.queryByText(/trace_id/)).toBeNull();
  });

  it("are shown in English from their code", async () => {
    await i18n.changeLanguage("en");
    renderTimeline(apiTurn);
    expect(
      screen.getByText(
        "OpenRouter did not accept the API key (HTTP 401). Check the key in the settings. Details: No auth credentials found",
      ),
    ).toBeTruthy();
    expect(screen.getByText(/openrouter\.ai did not answer within 180 seconds\. Try again\./)).toBeTruthy();
  });

  it("sort HTTP errors by what the user can do", async () => {
    await i18n.changeLanguage("ja");
    expect(failureText(i18n.t, "api_rate_limited", { service: "OpenRouter", status: 429 }, "x")).toBe(
      "OpenRouter がリクエストを制限しているか、アカウントのクレジットや利用枠が尽きています（HTTP 429）。少し待ってから試してください。",
    );
    expect(failureText(i18n.t, "api_server_error", { service: "OpenRouter", status: 502 }, "x")).toBe(
      "OpenRouter のサーバーでエラーが起きました（HTTP 502）。時間をおいて試してください。",
    );
    const noCredits = { service: "openrouter.ai", status: 402, detail: "Insufficient credits" };
    expect(failureText(i18n.t, "api_payment_required", noCredits, "x")).toBe(
      "openrouter.ai のアカウントのクレジットが足りません（HTTP 402）。クレジットを追加してから、もう一度試してください。詳細：Insufficient credits",
    );
    await i18n.changeLanguage("en");
    expect(failureText(i18n.t, "api_payment_required", noCredits, "x")).toBe(
      "The openrouter.ai account is out of credits (HTTP 402). Add credits, then try again. Details: Insufficient credits",
    );
    expect(
      failureText(i18n.t, "api_http_error", { service: "OpenRouter", status: 404, detail: "no such route" }, "x"),
    ).toBe("OpenRouter returned an error (HTTP 404). Details: no such route");
    expect(failureText(i18n.t, "api_unreachable", { service: "openrouter.ai" }, "x")).toBe(
      "Could not reach openrouter.ai. Check your network and the API URL in the settings, then try again.",
    );
  });

  it("say when the API failed partway through its answer", async () => {
    const params = { service: "openrouter.ai", detail: "Provider returned error" };
    await i18n.changeLanguage("ja");
    expect(failureText(i18n.t, "api_stream_failed", params, "x")).toBe(
      "openrouter.ai が応答の途中でエラーを返しました。もう一度試してください。詳細：Provider returned error",
    );
    await i18n.changeLanguage("en");
    expect(failureText(i18n.t, "api_stream_failed", params, "x")).toBe(
      "openrouter.ai sent an error partway through its answer. Try again. Details: Provider returned error",
    );
  });

  it("name the missing settings as the settings page does", async () => {
    await i18n.changeLanguage("ja");
    expect(failureText(i18n.t, "api_key_missing", { settings: "openrouter_api_key" }, "x")).toBe(
      "「画像生成」の「API キー」が設定されていません。設定画面で入力してください。",
    );
    expect(failureText(i18n.t, "setting_missing", { settings: "agent_llm_base_url, agent_llm_model" }, "x")).toBe(
      "「チャットアシスタント」の「API URL」、「チャットアシスタント」の「モデル」が入力されていません。設定画面で入力してください。",
    );
    await i18n.changeLanguage("en");
    expect(failureText(i18n.t, "setting_missing", { settings: "agent_llm_model" }, "x")).toBe(
      "Empty: Model (Chat assistant LLM). Fill it in on the settings page.",
    );
  });

  it("say why a reference was refused, and a stop", async () => {
    await i18n.changeLanguage("ja");
    expect(
      failureText(i18n.t, "reference_rejected", { reason: "reference_adult_explicit", detail: "explicit" }, "x"),
    ).toBe("この参考画像は使用できません：性的に露骨な画像とみられます。詳細：explicit");
    expect(failureText(i18n.t, "cancelled", {}, "Cancelled by the user.")).toBe("停止しました。");
    await i18n.changeLanguage("en");
    expect(failureText(i18n.t, "reference_rejected", { reason: "reference_unusable" }, "x")).toBe(
      "This reference image cannot be used: the character cannot be made out well enough for a mask design.",
    );
    expect(failureText(i18n.t, "no_references", {}, "x")).toBe(
      "There are no reference images yet. Upload at least one image of the character.",
    );
  });
});

describe("Notices", () => {
  it("that end a run have a translation in each language", async () => {
    // agent/runner.rs: a stop, too many steps, a model that kept answering nothing.
    for (const language of ["ja", "en", "zh-CN"]) {
      await i18n.changeLanguage(language);
      for (const code of ["cancelled", "step_limit", "empty_reply"]) {
        const key = `agent.notices.${code}`;
        expect(i18n.exists(key, { lng: language, fallbackLng: false }), `${language} ${key}`).toBe(true);
      }
    }
  });

  it("say that the answer was empty", async () => {
    await i18n.changeLanguage("ja");
    renderTimeline([
      { seq: 1, type: "user_message", text: "この設定画で進めてください" },
      { seq: 2, type: "notice", code: "empty_reply" },
    ]);
    expect(
      screen.getByText("AI の応答が空だったため、いったん停止しました。続けるには指示を送ってください。"),
    ).toBeTruthy();
  });
});
