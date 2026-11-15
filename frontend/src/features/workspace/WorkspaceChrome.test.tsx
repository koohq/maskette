import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "../../i18n";
import { DEFAULT_AGENT_WIDTH, MIN_AGENT_WIDTH } from "./agentWidth";
import { AgentResizeHandle, EditorTab, ImageLightbox } from "./WorkspaceChrome";

function ResizeHarness({ initial }: { initial: number }) {
  const [width, setWidth] = useState(initial);
  return <AgentResizeHandle onWidthChange={setWidth} width={width} />;
}

function LightboxHarness() {
  const [url, setUrl] = useState<string | null>(null);
  return (
    <>
      <button onClick={() => setUrl("/image.png")} type="button">
        thumbnail
      </button>
      {url ? <ImageLightbox onClose={() => setUrl(null)} url={url} /> : null}
    </>
  );
}

describe("workspace chrome", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("zh-CN");
    vi.stubGlobal("innerWidth", 1600);
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  afterAll(async () => {
    await i18n.changeLanguage("zh-CN");
  });

  it("resizes the agent panel from the keyboard", () => {
    render(<ResizeHarness initial={400} />);
    const splitter = screen.getByRole("separator", { name: "调整助手面板宽度" });
    expect(splitter).toHaveAttribute("tabindex", "0");
    expect(splitter).toHaveAttribute("aria-valuenow", "400");
    expect(splitter).toHaveAttribute("aria-valuemin", String(MIN_AGENT_WIDTH));

    fireEvent.keyDown(splitter, { key: "ArrowLeft" });
    expect(splitter).toHaveAttribute("aria-valuenow", "416");
    fireEvent.keyDown(splitter, { key: "ArrowRight", shiftKey: true });
    expect(splitter).toHaveAttribute("aria-valuenow", "352");
    for (let i = 0; i < 10; i += 1) fireEvent.keyDown(splitter, { key: "ArrowRight", shiftKey: true });
    expect(splitter).toHaveAttribute("aria-valuenow", String(MIN_AGENT_WIDTH));
    fireEvent.keyDown(splitter, { key: "Enter" });
    expect(splitter).toHaveAttribute("aria-valuenow", String(DEFAULT_AGENT_WIDTH));
  });

  it("renders a tab as open and close buttons, with the open one marked current", () => {
    const onOpen = vi.fn();
    const onClose = vi.fn();
    render(
      <EditorTab
        active
        id="v2"
        onClose={onClose}
        onOpen={onOpen}
        roleLabel="头壳正视图"
        thumbnailUrl="/v2.png"
        unsaved
      />,
    );

    expect(screen.queryByRole("tab")).toBeNull();
    // The unsaved mark is a word inside the open button, so it is part of the button's name.
    const open = screen.getByRole("button", { name: /^v2\s*头壳正视图\s*未保存$/ });
    expect(open).toHaveAttribute("aria-current", "true");
    expect(open.querySelector("img")).toHaveAttribute("src", "/v2.png");
    fireEvent.click(open);
    fireEvent.click(screen.getByRole("button", { name: "关闭 v2" }));
    expect(onOpen).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it.each([
    ["ja", "v2 を閉じる", "アシスタント欄の幅を変える", "画像プレビュー"],
    ["en", "Close v2", "Resize assistant panel", "Image preview"],
  ])("names the controls in %s", async (language, close, resize, preview) => {
    await i18n.changeLanguage(language);
    render(
      <>
        <EditorTab active={false} id="v2" onClose={vi.fn()} onOpen={vi.fn()} roleLabel="front" unsaved={false} />
        <AgentResizeHandle onWidthChange={vi.fn()} width={400} />
        <ImageLightbox onClose={vi.fn()} url="/image.png" />
      </>,
    );
    expect(screen.getByRole("button", { name: close })).toBeTruthy();
    expect(screen.getByRole("separator", { name: resize })).toBeTruthy();
    expect(screen.getByRole("dialog", { name: preview })).toBeTruthy();
  });

  it("moves focus into the image preview, closes it on Escape or a click, and returns focus", () => {
    render(<LightboxHarness />);
    const thumbnail = screen.getByRole("button", { name: "thumbnail" });
    thumbnail.focus();
    fireEvent.click(thumbnail);

    const preview = screen.getByRole("dialog", { name: "图片预览" });
    expect(preview).toHaveFocus();
    fireEvent.keyDown(preview, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(thumbnail).toHaveFocus();

    fireEvent.click(thumbnail);
    fireEvent.click(screen.getByRole("dialog", { name: "图片预览" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
