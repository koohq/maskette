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
    state: { references: [], images: [] },
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
        "Codex の CLI（codex）が見つかりません。インストールしてログインするか、設定で API キーの経路を選んでください。",
      ),
    ).toBeTruthy();
    expect(
      screen.getByText(
        "エラーが発生しました：Claude Code の利用上限に達しました。時間をおいて試すか、設定で API キーの経路を選んでください。",
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
        expect(i18n.exists(key), `${language} ${key}`).toBe(true);
      }
    }
  });
});
