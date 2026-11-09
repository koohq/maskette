import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import i18n from "../i18n";
import { ConfirmDialog, PromptDialog } from "./IdeDialog";

function ConfirmHarness({ onConfirm }: { onConfirm: () => void }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button onClick={() => setOpen(true)} type="button">
        open
      </button>
      {open ? (
        <ConfirmDialog
          confirmLabel="删除项目"
          danger
          message="确定删除？"
          onCancel={() => setOpen(false)}
          onConfirm={onConfirm}
          title="删除项目"
        />
      ) : null}
    </>
  );
}

describe("IdeDialog keyboard and roles", () => {
  beforeAll(async () => {
    await i18n.changeLanguage("zh-CN");
  });

  afterEach(() => cleanup());

  it("is a named modal dialog that takes focus, closes on Escape and gives focus back", () => {
    const onConfirm = vi.fn();
    render(<ConfirmHarness onConfirm={onConfirm} />);
    const opener = screen.getByRole("button", { name: "open" });
    opener.focus();
    fireEvent.click(opener);

    const dialog = screen.getByRole("dialog", { name: "删除项目" });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(dialog).toHaveFocus();

    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(opener).toHaveFocus();
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("keeps Tab inside the dialog", () => {
    render(<ConfirmHarness onConfirm={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "open" }));

    const cancel = screen.getByRole("button", { name: "取消" });
    const confirm = screen.getByRole("button", { name: "删除项目" });
    confirm.focus();
    fireEvent.keyDown(confirm, { key: "Tab" });
    expect(cancel).toHaveFocus();
    fireEvent.keyDown(cancel, { key: "Tab", shiftKey: true });
    expect(confirm).toHaveFocus();
  });

  it("focuses the prompt field and keeps an empty value blocked unless allowed", () => {
    render(<PromptDialog confirmLabel="保存" onCancel={vi.fn()} onConfirm={vi.fn()} title="重命名" />);
    expect(screen.getByRole("textbox", { name: "重命名" })).toHaveFocus();
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();

    cleanup();
    render(<PromptDialog allowEmpty confirmLabel="创建" onCancel={vi.fn()} onConfirm={vi.fn()} title="新建项目" />);
    expect(screen.getByRole("button", { name: "创建" })).toBeEnabled();
  });
});
