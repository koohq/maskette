import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterAll, afterEach, describe, expect, it, vi } from "vitest";
import i18n from "../../../i18n";
import type { ManualLandmarks } from "../deformation/landmarks";
import type { AnnotationMark } from "../deformation/recipe";
import { AnnotationLayer } from "./AnnotationLayer";

function mark(id: string, kind: AnnotationMark["kind"], extra: Partial<AnnotationMark> = {}): AnnotationMark {
  return { id, kind, x: 0.3, y: 0.3, width: 0.2, height: 0.1, endX: 0.5, endY: 0.5, note: "", ...extra };
}

const annotations = [
  mark("a", "pin"),
  mark("b", "arrow"),
  mark("c", "rect"),
  mark("d", "text", { text: "眼睛大一点" }),
];

const point = { x: 0.5, y: 0.5 };
const landmarks: ManualLandmarks = {
  leftEye: { x: 0.4, y: 0.4 },
  rightEye: { x: 0.6, y: 0.4 },
  chin: point,
  jawLeft: point,
  jawRight: point,
  mouthCenter: point,
  mouthLeft: point,
  mouthRight: point,
};

describe("AnnotationLayer marks", () => {
  afterEach(() => cleanup());

  afterAll(async () => {
    await i18n.changeLanguage("zh-CN");
  });

  it.each([
    ["zh-CN", ["标注 1", "箭头标注 2", "矩形标注 3", "文字标注 4: 眼睛大一点"], "关键点 左眼", "删除标注"],
    [
      "ja",
      ["注釈 1", "矢印の注釈 2", "四角形の注釈 3", "文字の注釈 4: 眼睛大一点"],
      "左目のキーポイント",
      "注釈を削除",
    ],
    [
      "en",
      ["Annotation 1", "Arrow annotation 2", "Rectangle annotation 3", "Text annotation 4: 眼睛大一点"],
      "Left eye landmark",
      "Delete annotation",
    ],
  ])("names each mark as a button in %s", async (language, markNames, landmarkName, deleteName) => {
    await i18n.changeLanguage(language);
    render(
      <AnnotationLayer
        annotations={annotations}
        height={400}
        landmarks={landmarks}
        selectedAnnotationId="a"
        showLandmarks
        width={400}
      />,
    );

    for (const name of markNames) expect(screen.getByRole("button", { name })).toBeTruthy();
    expect(screen.getByRole("button", { name: landmarkName })).toBeTruthy();
    expect(screen.getByRole("button", { name: deleteName })).toBeTruthy();
  });

  it("marks the selected mark and keeps the pointer handlers on the marks", async () => {
    await i18n.changeLanguage("zh-CN");
    const onAnnotationPointerDown = vi.fn();
    const onLandmarkPointerDown = vi.fn();
    render(
      <AnnotationLayer
        annotations={annotations}
        height={400}
        landmarks={landmarks}
        onAnnotationPointerDown={onAnnotationPointerDown}
        onLandmarkPointerDown={onLandmarkPointerDown}
        selectedAnnotationId="c"
        selectedLandmarkKey="leftEye"
        showLandmarks
        width={400}
      />,
    );

    const rect = screen.getByRole("button", { name: "矩形标注 3" });
    expect(rect).toHaveAttribute("data-selected", "true");
    expect(rect).toHaveAttribute("aria-current", "true");
    expect(screen.getByRole("button", { name: "标注 1" })).not.toHaveAttribute("aria-current");
    expect(screen.getByRole("button", { name: "关键点 左眼" })).toHaveAttribute("aria-current", "true");

    fireEvent.pointerDown(rect);
    fireEvent.pointerDown(screen.getByRole("button", { name: "关键点 右眼" }));
    expect(onAnnotationPointerDown).toHaveBeenCalledWith("c", expect.anything());
    expect(onLandmarkPointerDown).toHaveBeenCalledWith("rightEye", expect.anything());
  });

  it("selects a mark on focus, moves it with the arrow keys and deletes it with Delete", async () => {
    await i18n.changeLanguage("ja");
    const onAnnotationSelect = vi.fn();
    const onAnnotationMove = vi.fn();
    const onAnnotationDelete = vi.fn();
    render(
      <AnnotationLayer
        annotations={annotations}
        height={400}
        onAnnotationDelete={onAnnotationDelete}
        onAnnotationMove={onAnnotationMove}
        onAnnotationSelect={onAnnotationSelect}
        width={400}
      />,
    );

    const pin = screen.getByRole("button", { name: "注釈 1" });
    expect(pin).toHaveAccessibleDescription("矢印キーで動かせます（Shift で大きく）。Delete で消せます。");
    pin.focus();
    expect(onAnnotationSelect).toHaveBeenCalledWith("a");

    fireEvent.keyDown(pin, { key: "ArrowRight" });
    expect(onAnnotationMove).toHaveBeenLastCalledWith("a", { x: expect.closeTo(0.305), y: expect.closeTo(0.3) });
    fireEvent.keyDown(pin, { key: "ArrowUp", shiftKey: true });
    expect(onAnnotationMove).toHaveBeenLastCalledWith("a", { x: expect.closeTo(0.3), y: expect.closeTo(0.28) });
    // Shortcuts with Ctrl, Alt or Meta are left to the rest of the app.
    fireEvent.keyDown(pin, { ctrlKey: true, key: "ArrowLeft" });
    expect(onAnnotationMove).toHaveBeenCalledTimes(2);

    // The arrow has its own focus point at its start, named like the other marks.
    fireEvent.keyDown(screen.getByRole("button", { name: "矢印の注釈 2" }), { key: "Backspace" });
    expect(onAnnotationDelete).toHaveBeenCalledWith("b");
    fireEvent.keyDown(pin, { key: "Delete" });
    expect(onAnnotationDelete).toHaveBeenLastCalledWith("a");
  });

  it("moves focus to the next mark after a keyboard delete, or to the previous one at the end", async () => {
    await i18n.changeLanguage("ja");
    const onAnnotationSelect = vi.fn();
    render(
      <AnnotationLayer
        annotations={annotations}
        height={400}
        onAnnotationDelete={vi.fn()}
        onAnnotationSelect={onAnnotationSelect}
        width={400}
      />,
    );

    const arrow = screen.getByRole("button", { name: "矢印の注釈 2" });
    arrow.focus();
    fireEvent.keyDown(arrow, { key: "Delete" });
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "四角形の注釈 3" }));
    expect(onAnnotationSelect).toHaveBeenLastCalledWith("c");

    const last = screen.getByRole("button", { name: "文字の注釈 4: 眼睛大一点" });
    last.focus();
    fireEvent.keyDown(last, { key: "Backspace" });
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "四角形の注釈 3" }));
  });

  it("deletes the selected mark from its delete button on a click, not on the press", async () => {
    await i18n.changeLanguage("zh-CN");
    const onAnnotationDelete = vi.fn();
    render(
      <AnnotationLayer
        annotations={annotations}
        height={400}
        onAnnotationDelete={onAnnotationDelete}
        selectedAnnotationId="c"
        width={400}
      />,
    );

    const button = screen.getByRole("button", { name: "删除标注" });
    fireEvent.pointerDown(button);
    expect(onAnnotationDelete).not.toHaveBeenCalled();
    fireEvent.click(button);
    expect(onAnnotationDelete).toHaveBeenCalledWith("c");
  });

  it("moves a focused landmark with the arrow keys", async () => {
    await i18n.changeLanguage("en");
    const onLandmarkSelect = vi.fn();
    const onLandmarkMove = vi.fn();
    render(
      <AnnotationLayer
        height={400}
        landmarks={landmarks}
        onLandmarkMove={onLandmarkMove}
        onLandmarkSelect={onLandmarkSelect}
        showLandmarks
        width={400}
      />,
    );

    const leftEye = screen.getByRole("button", { name: "Left eye landmark" });
    expect(leftEye).toHaveAccessibleDescription("Arrow keys move it, farther with Shift.");
    leftEye.focus();
    expect(onLandmarkSelect).toHaveBeenCalledWith("leftEye");
    fireEvent.keyDown(leftEye, { key: "ArrowDown" });
    expect(onLandmarkMove).toHaveBeenLastCalledWith("leftEye", { x: expect.closeTo(0.4), y: expect.closeTo(0.405) });
    fireEvent.keyDown(leftEye, { key: "Delete" });
    expect(onLandmarkMove).toHaveBeenCalledTimes(1);
  });
});
