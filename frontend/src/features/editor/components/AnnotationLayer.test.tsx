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
    ["ja", ["注釈 1", "矢印の注釈 2", "矩形の注釈 3", "文字の注釈 4: 眼睛大一点"], "左目のキーポイント", "注釈を削除"],
    [
      "en",
      ["Annotation 1", "Arrow annotation 2", "Rectangle annotation 3", "Text annotation 4: 眼睛大一点"],
      "Left eye landmark",
      "Delete annotation",
    ],
  ])("names each mark as a graphic in %s", async (language, markNames, landmarkName, deleteName) => {
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

    for (const name of markNames) expect(screen.getByRole("img", { name })).toBeTruthy();
    expect(screen.getByRole("img", { name: landmarkName })).toBeTruthy();
    expect(screen.getByRole("button", { name: deleteName })).toBeTruthy();
  });

  it("keeps selection in data-selected and the pointer handlers on the marks", async () => {
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

    const rect = screen.getByRole("img", { name: "矩形标注 3" });
    expect(rect).toHaveAttribute("data-selected", "true");
    expect(rect).not.toHaveAttribute("aria-selected");
    expect(screen.getByRole("img", { name: "标注 1" })).not.toHaveAttribute("data-selected");
    expect(screen.getByRole("img", { name: "关键点 左眼" })).toHaveAttribute("data-selected", "true");

    fireEvent.pointerDown(rect);
    fireEvent.pointerDown(screen.getByRole("img", { name: "关键点 右眼" }));
    expect(onAnnotationPointerDown).toHaveBeenCalledWith("c", expect.anything());
    expect(onLandmarkPointerDown).toHaveBeenCalledWith("rightEye", expect.anything());
  });
});
