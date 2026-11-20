import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import i18n from "../../i18n";
import { type AgentEvent, applyEvent, type Conversation } from "./agentApi";
import { ChatTimeline } from "./ChatTimeline";

function conversation(approved: string | null = null): Conversation {
  return {
    id: "c",
    title: "",
    created_at: "",
    locale: "zh-CN",
    status: "idle",
    running: false,
    last_seq: 0,
    events: [],
    state: {
      references: [],
      missing_views: [],
      images: [],
      current_design_id: null,
      approved_design_id: null,
      current_front_id: "front-1",
      approved_front_id: approved,
    },
  };
}

const frontImage: AgentEvent = {
  seq: 3,
  type: "image",
  role: "front",
  image_id: "front-1",
  url: "/api/generated/a.webp",
  width: 800,
  height: 1100,
};

afterEach(cleanup);

beforeAll(async () => {
  await i18n.changeLanguage("zh-CN");
});

describe("applyEvent", () => {
  it("appends new events, ignores duplicates and applies patches in place", () => {
    const tool: AgentEvent = { seq: 1, type: "tool", tool: "generate_front_view", status: "running", progress: 0 };
    let events = applyEvent([], tool);
    events = applyEvent(events, tool);
    expect(events).toHaveLength(1);
    events = applyEvent(events, {
      seq: 2,
      type: "event_patch",
      target_seq: 1,
      patch: { status: "succeeded", progress: 100 },
    });
    expect(events).toHaveLength(1);
    expect(events[0]).toMatchObject({ status: "succeeded", progress: 100 });
  });
});

describe("ChatTimeline", () => {
  it("offers approval on the latest front view and sends it", () => {
    const onApprove = vi.fn();
    render(
      <ChatTimeline
        conversation={conversation()}
        events={[frontImage]}
        onApprove={onApprove}
        onOpenImage={vi.fn()}
        running={false}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /确认正视图/ }));
    expect(onApprove).toHaveBeenCalledWith("front-1");
  });

  it("hides approval while running and shows the approved badge afterwards", () => {
    const { rerender } = render(
      <ChatTimeline
        conversation={conversation()}
        events={[frontImage]}
        onApprove={vi.fn()}
        onOpenImage={vi.fn()}
        running
      />,
    );
    expect(screen.queryByRole("button", { name: /确认正视图/ })).toBeNull();
    rerender(
      <ChatTimeline
        conversation={conversation("front-1")}
        events={[frontImage]}
        onApprove={vi.fn()}
        onOpenImage={vi.fn()}
        running={false}
      />,
    );
    expect(screen.queryByRole("button", { name: /确认正视图/ })).toBeNull();
    expect(screen.getByText("已确认")).toBeTruthy();
  });

  it("shows an eye close-up full width, named, and with nothing to approve", () => {
    const eyes: AgentEvent = {
      seq: 5,
      type: "image",
      role: "eyes",
      image_id: "eyes-1",
      url: "/api/generated/e.webp",
      width: 1536,
      height: 768,
    };
    render(
      <ChatTimeline
        conversation={conversation("front-1")}
        events={[{ seq: 4, type: "tool", tool: "generate_eye_detail", status: "failed", error: "x" }, eyes]}
        onApprove={vi.fn()}
        onOpenImage={vi.fn()}
        running={false}
      />,
    );
    expect(screen.getByText("生成眼部特写")).toBeTruthy();
    const image = screen.getByRole("img", { name: "眼部特写" });
    expect(image.closest("li")?.className).not.toContain("max-w-60");
    expect(screen.queryByRole("button", { name: /确认/ })).toBeNull();
  });

  it("shows order notes in their own language and copies them for the maker", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    const notes: AgentEvent = {
      seq: 6,
      type: "order_notes",
      front_id: "front-1",
      language: "ja",
      views: ["front", "turnaround", "eyes"],
      sections: [
        { key: "eyes", text: "丸い目です。" },
        { key: "ears", text: "" },
        { key: "hair", text: "オレンジの長い髪です。" },
      ],
    };
    render(
      <ChatTimeline
        conversation={conversation("front-1")}
        events={[{ seq: 5, type: "tool", tool: "write_order_notes", status: "succeeded" }, notes]}
        onApprove={vi.fn()}
        onOpenImage={vi.fn()}
        running={false}
      />,
    );
    // The screen speaks the user's language; the notes and their labels the maker's.
    expect(screen.getByText("写定制说明")).toBeTruthy();
    expect(screen.getByText("定制说明")).toBeTruthy();
    expect(screen.getByText("日本語")).toBeTruthy();
    expect(screen.getByText("目")).toBeTruthy();
    expect(screen.queryByText("耳・角")).toBeNull();
    expect(screen.getByText("左右は、正面図を見たときの向き（向かって左・右）で書いています。")).toBeTruthy();
    expect(screen.queryByText(/之前确认的正视图/)).toBeNull();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "复制" }));
    });
    expect(writeText).toHaveBeenCalledWith(
      [
        "【目】丸い目です。",
        "【髪】オレンジの長い髪です。",
        "",
        "・左右は、正面図を見たときの向き（向かって左・右）で書いています。",
        "・画像どうしが食い違うときは、形は正面図、横と後ろは四面図、目は目の拡大図を優先してください。",
        "・色は画面で見た色です。素材での見え方はご相談ください。",
      ].join("\n"),
    );
    expect(screen.getByText("已复制")).toBeTruthy();
  });

  it("marks order notes written for an earlier front view", () => {
    const notes: AgentEvent = {
      seq: 6,
      type: "order_notes",
      front_id: "front-1",
      language: "en",
      views: ["front"],
      sections: [{ key: "eyes", text: "Round eyes." }],
    };
    render(
      <ChatTimeline
        conversation={conversation("front-2")}
        events={[notes]}
        onApprove={vi.fn()}
        onOpenImage={vi.fn()}
        running={false}
      />,
    );
    expect(screen.getByText(/之前确认的正视图/)).toBeTruthy();
    // Only the front view went in, so it is the only picture the priority line names.
    expect(screen.getByText("Where the pictures disagree, follow the front view for the shapes.")).toBeTruthy();
  });

  it("renders tool progress and failures", () => {
    render(
      <ChatTimeline
        conversation={conversation()}
        events={[
          { seq: 1, type: "tool", tool: "generate_turnaround", status: "failed", error: "No approved front view." },
        ]}
        onApprove={vi.fn()}
        onOpenImage={vi.fn()}
        running={false}
      />,
    );
    expect(screen.getByText("生成头壳四视图")).toBeTruthy();
    expect(screen.getByText("No approved front view.")).toBeTruthy();
  });

  it("ticks elapsed seconds on the thinking indicator", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-10-08T12:00:00Z"));
    render(
      <ChatTimeline
        conversation={conversation()}
        events={[{ seq: 1, type: "user_message", text: "hi", created_at: "2026-10-08T12:00:00Z" }]}
        onApprove={vi.fn()}
        onOpenImage={vi.fn()}
        running
      />,
    );
    expect(screen.getByText(/已用时 0:00/)).toBeTruthy();
    await act(async () => {
      vi.advanceTimersByTime(65_000);
    });
    expect(screen.getByText(/已用时 1:05/)).toBeTruthy();
    vi.useRealTimers();
  });

  it("ticks elapsed seconds on a running tool from its start timestamp", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-10-08T12:00:00Z"));
    render(
      <ChatTimeline
        conversation={conversation()}
        events={[
          {
            seq: 1,
            type: "tool",
            tool: "analyze_references",
            status: "running",
            progress: 10,
            phase: "analyzing",
            created_at: "2026-10-08T12:00:10Z",
          },
        ]}
        onApprove={vi.fn()}
        onOpenImage={vi.fn()}
        running
      />,
    );
    expect(screen.getByText(/已用时 0:00/)).toBeTruthy();
    await act(async () => {
      vi.advanceTimersByTime(70_000);
    });
    expect(screen.getByText(/已用时 1:00/)).toBeTruthy();
    vi.useRealTimers();
  });

  it("names the job step a running tool is on", () => {
    render(
      <ChatTimeline
        conversation={conversation()}
        events={[
          { seq: 1, type: "tool", tool: "generate_turnaround", status: "running", progress: 45, phase: "generating" },
          {
            seq: 2,
            type: "tool",
            tool: "generate_front_view",
            status: "running",
            progress: 50,
            phase: "some_new_step",
          },
        ]}
        onApprove={vi.fn()}
        onOpenImage={vi.fn()}
        running
      />,
    );
    expect(screen.getByText(/· 生成中/)).toBeTruthy();
    expect(screen.getByText(/· some_new_step/)).toBeTruthy();
    expect(screen.getAllByRole("progressbar")[0]?.getAttribute("aria-valuenow")).toBe("45");
  });

  it("shows the streamed reasoning token estimate on a running tool", () => {
    render(
      <ChatTimeline
        conversation={conversation()}
        events={[
          { seq: 1, type: "tool", tool: "analyze_references", status: "running", progress: 10, reasoning_tokens: 1050 },
        ]}
        onApprove={vi.fn()}
        onOpenImage={vi.fn()}
        running
      />,
    );
    expect(screen.getByText("已思考约 1050 tokens")).toBeTruthy();
  });

  it("offers a retry after a failed turn and drops it once a new message arrives", () => {
    const onRegenerate = vi.fn();
    const failedTurn: AgentEvent[] = [
      { seq: 1, type: "user_message", text: "帮我生成正面图" },
      { seq: 2, type: "error", message: "Agent LLM HTTP 500: boom" },
    ];
    const { rerender } = render(
      <ChatTimeline
        conversation={conversation()}
        events={failedTurn}
        onApprove={vi.fn()}
        onOpenImage={vi.fn()}
        onRegenerate={onRegenerate}
        running={false}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "重试" }));
    expect(onRegenerate).toHaveBeenCalledOnce();

    rerender(
      <ChatTimeline
        conversation={conversation()}
        events={[...failedTurn, { seq: 3, type: "user_message", text: "再来一次" }]}
        onApprove={vi.fn()}
        onOpenImage={vi.fn()}
        onRegenerate={onRegenerate}
        running={false}
      />,
    );
    expect(screen.queryByRole("button", { name: "重试" })).toBeNull();
  });
});
