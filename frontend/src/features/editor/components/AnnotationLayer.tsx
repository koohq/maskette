import { IconTrash } from "@tabler/icons-react";
import { type KeyboardEvent, type PointerEvent, useId } from "react";
import { useTranslation } from "react-i18next";
import { translateLiteralString } from "../../../i18n/literalTranslations";
import {
  type EyeRegion,
  getManualLandmarkPoint,
  type LandmarkPoint,
  type ManualLandmarkKey,
  type ManualLandmarks,
  resolveBrowLandmarks,
} from "../deformation/landmarks";
import type { AnnotationMark, DetailRegion, LiquifyStroke } from "../deformation/recipe";

export type AnnotationLayerProps = {
  annotations?: AnnotationMark[] | undefined;
  brushPreview?: { active: boolean; mode: "scale" | "warp"; radius: number; x: number; y: number } | null | undefined;
  debugFaceBox?: { height: number; width: number; x: number; y: number } | null | undefined;
  debugHrnetBox?: { height: number; width: number; x: number; y: number } | null | undefined;
  debugLandmarks?: Array<LandmarkPoint & { index: number; score?: number }> | undefined;
  detailRegions?: DetailRegion[] | undefined;
  eyeRegionScale?: number | undefined;
  height: number;
  landmarks?: ManualLandmarks | undefined;
  liquifySymmetryAxis?: number | undefined;
  liquifyStrokes?: LiquifyStroke[] | undefined;
  /** Draw brush and stroke circles at their true radius in preview pixels instead of a clamped hint size. */
  exactBrushRadius?: boolean | undefined;
  secondaryLandmarks?: LandmarkPoint[] | undefined;
  selectedAnnotationId?: string | null | undefined;
  selectedLandmarkKey?: ManualLandmarkKey | null | undefined;
  showLandmarkDebug?: boolean | undefined;
  showLandmarks?: boolean | undefined;
  showLiquifySymmetryAxis?: boolean | undefined;
  showLiquifyStrokes?: boolean | undefined;
  showSecondaryLandmarks?: boolean | undefined;
  visible?: boolean | undefined;
  width: number;
  onAnnotationHandlePointerDown?:
    | ((annotationId: string, handle: string, event: PointerEvent<HTMLDivElement>) => void)
    | undefined;
  onAnnotationPointerDown?: ((annotationId: string, event: PointerEvent<HTMLElement>) => void) | undefined;
  onAnnotationDelete?: ((annotationId: string) => void) | undefined;
  /** A mark got the keyboard focus: select it as a click would. */
  onAnnotationSelect?: ((annotationId: string) => void) | undefined;
  /** An arrow key moved a focused mark; `point` is its new `x`, `y`. */
  onAnnotationMove?: ((annotationId: string, point: { x: number; y: number }) => void) | undefined;
  onLandmarkPointerDown?: ((key: ManualLandmarkKey, event: PointerEvent<HTMLElement>) => void) | undefined;
  onLandmarkSelect?: ((key: ManualLandmarkKey) => void) | undefined;
  onLandmarkMove?: ((key: ManualLandmarkKey, point: { x: number; y: number }) => void) | undefined;
};

/** How far an arrow key moves a focused mark, as a share of the image: half a percent, four times that with Shift. */
const ARROW_STEP = 0.005;
const ARROW_STEP_LARGE = 0.02;

function arrowKeyOffset(event: KeyboardEvent<HTMLElement>): { x: number; y: number } | null {
  if (event.altKey || event.ctrlKey || event.metaKey) return null;
  const step = event.shiftKey ? ARROW_STEP_LARGE : ARROW_STEP;
  if (event.key === "ArrowLeft") return { x: -step, y: 0 };
  if (event.key === "ArrowRight") return { x: step, y: 0 };
  if (event.key === "ArrowUp") return { x: 0, y: -step };
  if (event.key === "ArrowDown") return { x: 0, y: step };
  return null;
}

/** Resets the button look so a mark keeps its own drawing; `.editor-mark` adds the focus ring. */
const markButtonReset = {
  font: "inherit",
  margin: 0,
  padding: 0,
} as const;

const landmarkLabels: Array<{ key: ManualLandmarkKey; label: string }> = [
  { key: "leftEye", label: "左眼" },
  { key: "rightEye", label: "右眼" },
  { key: "chin", label: "下巴" },
  { key: "jawLeft", label: "左下颌" },
  { key: "jawRight", label: "右下颌" },
  { key: "mouthLeft", label: "左嘴角" },
  { key: "mouthCenter", label: "嘴巴" },
  { key: "mouthRight", label: "右嘴角" },
  { key: "leftBrowInner", label: "左眉头" },
  { key: "leftBrowPeak", label: "左眉峰" },
  { key: "leftBrowOuter", label: "左眉尾" },
  { key: "rightBrowInner", label: "右眉头" },
  { key: "rightBrowPeak", label: "右眉峰" },
  { key: "rightBrowOuter", label: "右眉尾" },
];

function toPercent(value: number) {
  return `${Math.min(1, Math.max(0, value)) * 100}%`;
}

function normalizeDebugPoint(point: LandmarkPoint, width: number, height: number) {
  return {
    x: point.x > 1 ? point.x / Math.max(1, width) : point.x,
    y: point.y > 1 ? point.y / Math.max(1, height) : point.y,
  };
}

function scaledEyeRegion(region: EyeRegion | undefined, scaleValue = 20) {
  if (!region) return null;
  const multiplier = Math.min(1.4, Math.max(0.5, 1 + scaleValue / 100));
  return {
    radiusBottomY: region.radiusBottomY * multiplier,
    radiusTopY: region.radiusTopY * multiplier,
    radiusX: region.radiusX * multiplier,
  };
}

function annotationKind(annotation: AnnotationMark) {
  if (annotation.kind === "box") return "rect";
  if (annotation.kind === "callout") return "pin";
  return annotation.kind;
}

function annotationColor(annotation: AnnotationMark) {
  return annotation.color || "#ef4444";
}

function selectedActionPosition(annotation: AnnotationMark) {
  const kind = annotationKind(annotation);
  if (kind === "arrow" || kind === "rect") {
    return {
      x: Math.max(annotation.x, annotation.endX),
      y: Math.min(annotation.y, annotation.endY),
    };
  }
  if (kind === "text") {
    return {
      x: annotation.x + annotation.width / 2,
      y: annotation.y - annotation.height / 2,
    };
  }
  return {
    x: annotation.x,
    y: annotation.y,
  };
}

export function AnnotationLayer({
  annotations = [],
  brushPreview,
  debugFaceBox,
  debugHrnetBox,
  debugLandmarks = [],
  detailRegions = [],
  eyeRegionScale,
  height,
  landmarks,
  liquifySymmetryAxis = 0.5,
  liquifyStrokes = [],
  exactBrushRadius = false,
  secondaryLandmarks = [],
  selectedLandmarkKey,
  selectedAnnotationId,
  showLandmarkDebug = false,
  showLandmarks = false,
  showLiquifySymmetryAxis = false,
  showLiquifyStrokes = false,
  showSecondaryLandmarks = false,
  visible = true,
  width,
  onAnnotationHandlePointerDown,
  onAnnotationPointerDown,
  onAnnotationDelete,
  onAnnotationMove,
  onAnnotationSelect,
  onLandmarkPointerDown,
  onLandmarkMove,
  onLandmarkSelect,
}: AnnotationLayerProps) {
  const { i18n } = useTranslation();
  const annotationHintId = useId();
  const landmarkHintId = useId();
  if (!visible) return null;
  const showLandmarkMarks = Boolean(showLandmarks && landmarks);

  const stageTestId = showLandmarks ? "landmark-stage" : showLiquifyStrokes ? "liquify-stage" : "annotation-stage";
  const visibleLiquifyStrokes = showLiquifyStrokes
    ? liquifyStrokes.filter((stroke) => stroke.mode === "scale").slice(-1)
    : [];
  const firstVisibleLiquifyStroke = visibleLiquifyStrokes[0];
  const visibleLiquifyStartIndex = firstVisibleLiquifyStroke ? liquifyStrokes.indexOf(firstVisibleLiquifyStroke) : 0;

  return (
    <div
      data-height={Math.max(1, Math.round(height))}
      data-testid={stageTestId}
      data-width={Math.max(1, Math.round(width))}
      style={{
        inset: 0,
        pointerEvents: "none",
        position: "absolute",
        zIndex: 3,
      }}
    >
      {annotations.length > 0 ? (
        <span hidden id={annotationHintId}>
          {translateLiteralString("方向键移动，按住 Shift 移动更多，Delete 删除。", i18n.language)}
        </span>
      ) : null}
      {showLandmarkMarks ? (
        <span hidden id={landmarkHintId}>
          {translateLiteralString("方向键移动，按住 Shift 移动更多。", i18n.language)}
        </span>
      ) : null}
      {annotations.map((annotation, index) => {
        const kind = annotationKind(annotation);
        const selected = selectedAnnotationId === annotation.id;
        const color = annotationColor(annotation);
        const label = String(index + 1);
        const markName = (kindLabel: string) => translateLiteralString(`${kindLabel} ${label}`, i18n.language);
        // Each mark is a button: Tab reaches it, focus selects it like a click, the arrow keys move it and Delete
        // removes it. aria-current tells which mark is selected; the drawing keeps data-selected for the styles.
        const markKeyboard = {
          "aria-current": selected || undefined,
          "aria-describedby": annotationHintId,
          className: "editor-mark",
          onFocus: () => onAnnotationSelect?.(annotation.id),
          onKeyDown: (event: KeyboardEvent<HTMLElement>) => {
            const offset = arrowKeyOffset(event);
            if (offset) {
              event.preventDefault();
              onAnnotationMove?.(annotation.id, { x: annotation.x + offset.x, y: annotation.y + offset.y });
              return;
            }
            if (event.key === "Delete" || event.key === "Backspace") {
              event.preventDefault();
              onAnnotationDelete?.(annotation.id);
            }
          },
          type: "button",
        } as const;
        const rectLeft = Math.min(annotation.x, annotation.endX);
        const rectTop = Math.min(annotation.y, annotation.endY);
        const rectWidth = Math.max(0.01, Math.abs(annotation.endX - annotation.x) || annotation.width);
        const rectHeight = Math.max(0.01, Math.abs(annotation.endY - annotation.y) || annotation.height);
        const handleStyle = {
          background: "#e0f2fe",
          border: `2px solid ${color}`,
          boxShadow: "0 4px 10px rgba(15, 23, 42, 0.35)",
          height: 14,
          pointerEvents: "auto" as const,
          position: "absolute" as const,
          transform: "translate(-50%, -50%)",
          width: 14,
          zIndex: 5,
        };
        const selectedAction = selectedActionPosition(annotation);
        const deleteButton = selected ? (
          <button
            aria-label={translateLiteralString("删除标注", i18n.language)}
            data-testid={`annotation-delete-${annotation.id}`}
            // Delete on click, so Enter and Space work too; the pointer press must not reach the stage below.
            onClick={() => onAnnotationDelete?.(annotation.id)}
            onPointerDown={(event) => event.stopPropagation()}
            style={{
              alignItems: "center",
              background: "var(--kb-panel)",
              border: "1px solid var(--kb-line)",
              boxShadow: "var(--kb-hard-shadow-sm)",
              color: "var(--kb-muted-red)",
              cursor: "pointer",
              display: "flex",
              height: 28,
              justifyContent: "center",
              left: toPercent(selectedAction.x),
              padding: 0,
              pointerEvents: "auto",
              position: "absolute",
              top: toPercent(selectedAction.y),
              transform: "translate(8px, -36px)",
              width: 28,
              zIndex: 8,
            }}
            type="button"
          >
            <IconTrash size={16} />
          </button>
        ) : null;

        if (kind === "arrow") {
          return (
            <div key={annotation.id}>
              {/* The arrow is drawn in an SVG over the whole stage; a small button at its start takes the focus. */}
              <svg
                aria-hidden="true"
                data-selected={selected || undefined}
                data-testid={`annotation-arrow-${index + 1}`}
                style={{
                  height: "100%",
                  inset: 0,
                  overflow: "visible",
                  pointerEvents: "auto",
                  position: "absolute",
                  width: "100%",
                }}
                viewBox="0 0 100 100"
                preserveAspectRatio="none"
              >
                <defs>
                  <marker
                    id={`annotation-arrow-head-${annotation.id}`}
                    markerHeight="7"
                    markerWidth="7"
                    orient="auto"
                    refX="6"
                    refY="3.5"
                  >
                    <polygon fill={color} points="0 0, 7 3.5, 0 7" />
                  </marker>
                </defs>
                <line
                  markerEnd={`url(#annotation-arrow-head-${annotation.id})`}
                  pointerEvents="none"
                  stroke={color}
                  strokeLinecap="round"
                  strokeWidth={Math.max(0.45, (annotation.strokeWidth ?? 4) / 8)}
                  x1={annotation.x * 100}
                  x2={annotation.endX * 100}
                  y1={annotation.y * 100}
                  y2={annotation.endY * 100}
                />
                <line
                  data-testid={`annotation-arrow-hit-${index + 1}`}
                  onPointerDown={(event) => {
                    event.stopPropagation();
                    onAnnotationPointerDown?.(annotation.id, event as unknown as PointerEvent<HTMLElement>);
                  }}
                  pointerEvents="stroke"
                  stroke="transparent"
                  strokeLinecap="round"
                  strokeWidth={4}
                  x1={annotation.x * 100}
                  x2={annotation.endX * 100}
                  y1={annotation.y * 100}
                  y2={annotation.endY * 100}
                />
              </svg>
              <button
                {...markKeyboard}
                aria-label={markName("箭头标注")}
                className="editor-mark editor-mark-grip"
                data-testid={`annotation-arrow-focus-${index + 1}`}
                style={{
                  ...markButtonReset,
                  background: "#e0f2fe",
                  border: `2px solid ${color}`,
                  height: 14,
                  left: toPercent(annotation.x),
                  // Keyboard only: the pointer keeps dragging the arrow's line.
                  pointerEvents: "none",
                  position: "absolute",
                  top: toPercent(annotation.y),
                  transform: "translate(-50%, -50%)",
                  width: 14,
                }}
              />
              {selected ? (
                <div
                  data-testid={`annotation-handle-${annotation.id}-end`}
                  onPointerDown={(event) => {
                    event.stopPropagation();
                    onAnnotationHandlePointerDown?.(annotation.id, "end", event);
                  }}
                  style={{
                    ...handleStyle,
                    cursor: "crosshair",
                    left: toPercent(annotation.endX),
                    top: toPercent(annotation.endY),
                  }}
                />
              ) : null}
              {deleteButton}
            </div>
          );
        }

        if (kind === "rect") {
          return (
            <div key={annotation.id}>
              <button
                {...markKeyboard}
                aria-label={markName("矩形标注")}
                data-selected={selected || undefined}
                data-testid={`annotation-rect-${index + 1}`}
                onPointerDown={(event) => {
                  event.stopPropagation();
                  onAnnotationPointerDown?.(annotation.id, event);
                }}
                style={{
                  ...markButtonReset,
                  background: selected ? `${color}22` : `${color}12`,
                  border: `${selected ? 3 : 2}px solid ${color}`,
                  boxShadow: selected ? "0 0 0 3px rgba(224, 242, 254, 0.22)" : "none",
                  cursor: "grab",
                  height: toPercent(rectHeight),
                  left: toPercent(rectLeft),
                  pointerEvents: "auto",
                  position: "absolute",
                  top: toPercent(rectTop),
                  width: toPercent(rectWidth),
                }}
              />
              {selected
                ? (
                    [
                      ["nw", rectLeft, rectTop],
                      ["ne", rectLeft + rectWidth, rectTop],
                      ["sw", rectLeft, rectTop + rectHeight],
                      ["se", rectLeft + rectWidth, rectTop + rectHeight],
                    ] as const
                  ).map(([handle, x, y]) => (
                    <div
                      data-testid={`annotation-handle-${annotation.id}-${handle}`}
                      key={handle}
                      onPointerDown={(event) => {
                        event.stopPropagation();
                        onAnnotationHandlePointerDown?.(annotation.id, handle, event);
                      }}
                      style={{
                        ...handleStyle,
                        cursor: `${handle}-resize`,
                        left: toPercent(x),
                        top: toPercent(y),
                      }}
                    />
                  ))
                : null}
              {deleteButton}
            </div>
          );
        }

        if (kind === "text") {
          return (
            <div key={annotation.id}>
              <button
                {...markKeyboard}
                aria-label={
                  annotation.text?.trim() ? `${markName("文字标注")}: ${annotation.text.trim()}` : markName("文字标注")
                }
                data-selected={selected || undefined}
                data-testid={`annotation-text-${index + 1}`}
                onPointerDown={(event) => {
                  event.stopPropagation();
                  onAnnotationPointerDown?.(annotation.id, event);
                }}
                style={{
                  ...markButtonReset,
                  background: "transparent",
                  border: selected ? `2px solid ${color}` : "2px solid transparent",
                  color,
                  cursor: "grab",
                  fontSize: annotation.fontSize ?? 24,
                  fontWeight: 900,
                  left: toPercent(annotation.x),
                  lineHeight: 1.15,
                  minHeight: 24,
                  minWidth: 34,
                  pointerEvents: "auto",
                  position: "absolute",
                  textAlign: "left",
                  top: toPercent(annotation.y),
                  transform: "translate(-50%, -50%)",
                  whiteSpace: "pre-wrap",
                  wordBreak: "break-word",
                  width: toPercent(annotation.width),
                }}
              >
                {annotation.text?.trim() || "文字"}
              </button>
              {selected ? (
                <div
                  data-testid={`annotation-handle-${annotation.id}-se`}
                  onPointerDown={(event) => {
                    event.stopPropagation();
                    onAnnotationHandlePointerDown?.(annotation.id, "se", event);
                  }}
                  style={{
                    ...handleStyle,
                    cursor: "se-resize",
                    left: toPercent(annotation.x + annotation.width / 2),
                    top: toPercent(annotation.y + annotation.height / 2),
                  }}
                />
              ) : null}
              {deleteButton}
            </div>
          );
        }

        return (
          <div key={annotation.id}>
            <button
              {...markKeyboard}
              aria-label={markName("标注")}
              data-selected={selected || undefined}
              data-testid={`annotation-pin-${index + 1}`}
              onPointerDown={(event) => {
                event.stopPropagation();
                onAnnotationPointerDown?.(annotation.id, event);
              }}
              style={{
                ...markButtonReset,
                alignItems: "center",
                background: selected ? "var(--kb-dirty-yellow)" : color,
                border: "3px solid rgba(255, 255, 255, 0.92)",
                borderRadius: 0,
                boxShadow: selected
                  ? "0 0 0 4px rgba(34, 211, 238, 0.24), var(--kb-hard-shadow-sm)"
                  : "var(--kb-hard-shadow-sm)",
                color: "#fff",
                cursor: "grab",
                display: "flex",
                fontSize: 14,
                fontWeight: 900,
                height: 30,
                justifyContent: "center",
                left: toPercent(annotation.x),
                lineHeight: 1,
                pointerEvents: "auto",
                position: "absolute",
                top: toPercent(annotation.y),
                transform: "translate(-50%, -50%)",
                width: 30,
              }}
            >
              {index + 1}
            </button>
            {deleteButton}
          </div>
        );
      })}

      {detailRegions.map((region) => {
        const diameter = `${Math.max(42, region.radius * 180)}px`;

        return (
          <div
            data-testid={`detail-region-${region.id}`}
            key={region.id}
            style={{
              background: "rgba(45, 212, 191, 0.12)",
              border: "2px dashed #2dd4bf",
              borderRadius: 999,
              height: diameter,
              left: toPercent(region.x),
              position: "absolute",
              top: toPercent(region.y),
              transform: "translate(-50%, -50%)",
              width: diameter,
            }}
          />
        );
      })}

      {showLandmarkMarks && landmarks
        ? landmarkLabels.map(({ key, label }) => {
            const landmark = getManualLandmarkPoint(landmarks, key);
            if (!landmark) return null;
            const selected = selectedLandmarkKey === key;
            const brow = key.includes("Brow");
            // Brows without detection are estimated from the eyes; show them fainter until confirmed.
            const estimated = brow && !landmarks.brows;

            return (
              // A button like the annotation marks above: focus selects it and the arrow keys move it.
              <button
                aria-current={selected || undefined}
                aria-describedby={landmarkHintId}
                aria-label={translateLiteralString(`关键点 ${label}`, i18n.language)}
                className="editor-mark"
                data-estimated={estimated || undefined}
                data-selected={selected || undefined}
                data-testid={`landmark-${key}`}
                key={key}
                onFocus={() => onLandmarkSelect?.(key)}
                onKeyDown={(event) => {
                  const offset = arrowKeyOffset(event);
                  if (!offset) return;
                  event.preventDefault();
                  onLandmarkMove?.(key, { x: landmark.x + offset.x, y: landmark.y + offset.y });
                }}
                onPointerDown={(event) => {
                  event.stopPropagation();
                  onLandmarkPointerDown?.(key, event);
                }}
                type="button"
                style={{
                  ...markButtonReset,
                  alignItems: "center",
                  background: "transparent",
                  border: 0,
                  cursor: "grab",
                  display: "flex",
                  height: 30,
                  justifyContent: "center",
                  left: toPercent(landmark.x),
                  pointerEvents: "auto",
                  position: "absolute",
                  top: toPercent(landmark.y),
                  transform: "translate(-50%, -50%)",
                  width: 30,
                }}
              >
                <span
                  style={{
                    background: brow ? "rgb(20, 160, 140)" : "var(--kb-accent)",
                    border: estimated ? "2px dashed var(--kb-panel)" : "2px solid var(--kb-panel)",
                    borderRadius: 999,
                    boxShadow: selected
                      ? `0 0 0 7px ${brow ? "rgba(20, 160, 140, 0.3)" : "rgba(209, 82, 47, 0.28)"}, var(--kb-hard-shadow-sm)`
                      : `0 0 0 5px ${brow ? "rgba(20, 160, 140, 0.16)" : "rgba(209, 82, 47, 0.16)"}`,
                    display: "block",
                    height: brow ? 12 : 18,
                    opacity: estimated ? 0.45 : 0.72,
                    width: brow ? 12 : 18,
                  }}
                />
              </button>
            );
          })
        : null}

      {showLandmarks && landmarks?.eyeRegions
        ? (
            [
              ["left", landmarks.leftEye, scaledEyeRegion(landmarks.eyeRegions.left, eyeRegionScale)],
              ["right", landmarks.rightEye, scaledEyeRegion(landmarks.eyeRegions.right, eyeRegionScale)],
            ] as const
          ).map(([side, center, region]) => {
            if (!region) return null;
            const regionRadiusY = Math.max(region.radiusTopY, region.radiusBottomY);

            return (
              <div
                data-testid={`eye-region-${side}`}
                key={side}
                style={{
                  border: "2px dashed rgba(34, 211, 238, 0.78)",
                  borderRadius: "50%",
                  height: toPercent(regionRadiusY * 2),
                  left: toPercent(center.x),
                  pointerEvents: "none",
                  position: "absolute",
                  top: toPercent(center.y),
                  transform: "translate(-50%, -50%)",
                  width: toPercent(region.radiusX * 2),
                }}
              />
            );
          })
        : null}

      {showLandmarks && landmarks
        ? (() => {
            const brows = resolveBrowLandmarks(landmarks);
            const sides = [brows.left, brows.right];
            return (
              <svg
                aria-hidden="true"
                data-testid="brow-lines"
                preserveAspectRatio="none"
                style={{ height: "100%", inset: 0, pointerEvents: "none", position: "absolute", width: "100%" }}
                viewBox="0 0 100 100"
              >
                {sides.map((brow, index) => (
                  <polyline
                    fill="none"
                    // biome-ignore lint/suspicious/noArrayIndexKey: the two brows always render in the same left, right order
                    key={index}
                    points={[brow.inner, brow.peak, brow.outer]
                      .map((point) => `${point.x * 100},${point.y * 100}`)
                      .join(" ")}
                    stroke="rgba(52, 211, 153, 0.85)"
                    strokeWidth={2}
                    vectorEffect="non-scaling-stroke"
                  />
                ))}
              </svg>
            );
          })()
        : null}

      {showLiquifySymmetryAxis ? (
        <div
          aria-hidden="true"
          data-axis={Math.min(1, Math.max(0, liquifySymmetryAxis)).toFixed(4)}
          data-testid="liquify-symmetry-axis"
          style={{
            borderLeft: "2px dashed rgba(34, 211, 238, 0.95)",
            bottom: 0,
            boxShadow: "0 0 0 1px rgba(15, 23, 42, 0.5), 0 0 12px rgba(34, 211, 238, 0.45)",
            left: toPercent(liquifySymmetryAxis),
            pointerEvents: "none",
            position: "absolute",
            top: 0,
            transform: "translateX(-1px)",
            zIndex: 6,
          }}
        >
          <span
            style={{
              background: "rgba(34, 211, 238, 0.9)",
              border: "2px solid rgba(15, 23, 42, 0.78)",
              boxShadow: "var(--kb-hard-shadow-sm)",
              display: "block",
              height: 14,
              left: 0,
              position: "absolute",
              top: 8,
              transform: "translateX(-50%)",
              width: 14,
            }}
          />
        </div>
      ) : null}

      {showLandmarkDebug && debugFaceBox ? (
        <div
          data-testid="landmark-debug-face-box"
          style={{
            border: "2px dashed rgba(34, 197, 94, 0.7)",
            height: toPercent(debugFaceBox.height),
            left: toPercent(debugFaceBox.x),
            pointerEvents: "none",
            position: "absolute",
            top: toPercent(debugFaceBox.y),
            width: toPercent(debugFaceBox.width),
          }}
        />
      ) : null}

      {showLandmarkDebug && debugHrnetBox ? (
        <div
          data-testid="landmark-debug-hrnet-box"
          style={{
            border: "2px dotted rgba(96, 165, 250, 0.72)",
            height: toPercent(debugHrnetBox.height),
            left: toPercent(debugHrnetBox.x),
            pointerEvents: "none",
            position: "absolute",
            top: toPercent(debugHrnetBox.y),
            width: toPercent(debugHrnetBox.width),
          }}
        />
      ) : null}

      {showLandmarkDebug
        ? debugLandmarks.map((point) => {
            const normalized = normalizeDebugPoint(point, width, height);

            return (
              <div
                data-testid={`landmark-debug-point-${point.index}`}
                key={point.index}
                style={{
                  height: 7,
                  left: toPercent(normalized.x),
                  pointerEvents: "none",
                  position: "absolute",
                  top: toPercent(normalized.y),
                  transform: "translate(-50%, -50%)",
                  width: 7,
                  zIndex: 1,
                }}
              >
                <span
                  style={{
                    background: "rgba(250, 204, 21, 0.72)",
                    border: "1px solid rgba(24, 24, 27, 0.72)",
                    borderRadius: 999,
                    display: "block",
                    height: "100%",
                    width: "100%",
                  }}
                />
                <span
                  style={{
                    color: "rgba(250, 204, 21, 0.88)",
                    fontSize: 9,
                    left: "50%",
                    lineHeight: 1,
                    position: "absolute",
                    top: 8,
                    transform: "translateX(-50%)",
                  }}
                >
                  {point.index}
                </span>
              </div>
            );
          })
        : null}

      {showSecondaryLandmarks
        ? secondaryLandmarks.map((point, index) => (
            <span
              data-testid={`landmark-secondary-${index}`}
              // biome-ignore lint/suspicious/noArrayIndexKey: detected points can share a position; the index keeps the keys unique
              key={`${point.x}-${point.y}-${index}`}
              style={{
                background: "rgba(250, 204, 21, 0.52)",
                borderRadius: 999,
                height: 6,
                left: toPercent(point.x),
                pointerEvents: "none",
                position: "absolute",
                top: toPercent(point.y),
                transform: "translate(-50%, -50%)",
                width: 6,
              }}
            />
          ))
        : null}

      {showLiquifyStrokes
        ? visibleLiquifyStrokes.map((stroke, index) => {
            const size = exactBrushRadius
              ? stroke.radius * 2
              : stroke.mode === "scale"
                ? Math.max(24, Math.min(320, stroke.radius * 2))
                : Math.max(18, Math.min(180, stroke.radius * 1.4));
            const isScaleStroke = stroke.mode === "scale";

            return (
              <div
                data-testid={`liquify-stroke-${visibleLiquifyStartIndex + index + 1}`}
                // biome-ignore lint/suspicious/noArrayIndexKey: strokes have no id; the index keeps identical strokes apart
                key={`${stroke.mode}-${stroke.x}-${stroke.y}-${stroke.radius}-${stroke.scale ?? ""}-${visibleLiquifyStartIndex + index}`}
                style={{
                  background: isScaleStroke
                    ? "radial-gradient(circle, rgba(216, 180, 254, 0.2) 0%, rgba(216, 180, 254, 0.08) 46%, rgba(216, 180, 254, 0.02) 72%, rgba(216, 180, 254, 0) 100%)"
                    : "radial-gradient(circle, rgba(103, 232, 249, 0.18) 0%, rgba(103, 232, 249, 0.07) 52%, rgba(103, 232, 249, 0) 100%)",
                  border: isScaleStroke
                    ? "2px dashed rgba(216, 180, 254, 0.92)"
                    : "2px dashed rgba(103, 232, 249, 0.92)",
                  borderRadius: 999,
                  boxShadow: "0 0 0 3px rgba(15, 23, 42, 0.18)",
                  height: size,
                  left: toPercent(stroke.x),
                  position: "absolute",
                  top: toPercent(stroke.y),
                  transform: "translate(-50%, -50%)",
                  width: size,
                }}
              />
            );
          })
        : null}

      {brushPreview ? (
        <div
          aria-hidden="true"
          data-active={brushPreview.active ? "true" : "false"}
          data-mode={brushPreview.mode}
          data-testid="liquify-brush-preview"
          style={{
            background: brushPreview.active
              ? "radial-gradient(circle, rgba(34, 197, 94, 0.24) 0%, rgba(34, 197, 94, 0.1) 55%, rgba(34, 197, 94, 0) 100%)"
              : brushPreview.mode === "scale"
                ? "radial-gradient(circle, rgba(216, 180, 254, 0.18) 0%, rgba(216, 180, 254, 0.08) 55%, rgba(216, 180, 254, 0) 100%)"
                : "radial-gradient(circle, rgba(103, 232, 249, 0.2) 0%, rgba(103, 232, 249, 0.08) 55%, rgba(103, 232, 249, 0) 100%)",
            border: brushPreview.active
              ? "2px solid rgba(34, 197, 94, 0.95)"
              : brushPreview.mode === "scale"
                ? "2px dashed rgba(216, 180, 254, 0.95)"
                : "2px dashed rgba(103, 232, 249, 0.95)",
            borderRadius: 999,
            boxShadow: "0 0 0 3px rgba(15, 23, 42, 0.18)",
            height: exactBrushRadius ? brushPreview.radius * 2 : Math.max(18, Math.min(360, brushPreview.radius * 2)),
            left: toPercent(brushPreview.x),
            pointerEvents: "none",
            position: "absolute",
            top: toPercent(brushPreview.y),
            transform: "translate(-50%, -50%)",
            width: exactBrushRadius ? brushPreview.radius * 2 : Math.max(18, Math.min(360, brushPreview.radius * 2)),
            zIndex: 7,
          }}
        />
      ) : null}
    </div>
  );
}
