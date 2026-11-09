/** Small pieces of the workspace frame, kept apart from WorkspacePage so they can be tested on their own. */

import { IconPhoto, IconX } from "@tabler/icons-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useModalFocus } from "../../ui/useModalFocus";
import { clampAgentWidth, DEFAULT_AGENT_WIDTH, MAX_AGENT_WIDTH, MIN_AGENT_WIDTH } from "./agentWidth";

/** Pixels one arrow key press moves the splitter; Shift moves it four times as far. */
const AGENT_RESIZE_STEP = 16;

/**
 * Splitter west of the agent panel. Drag it, or focus it and use the arrow keys (Left widens the panel);
 * double-click or Enter resets the width.
 */
export function AgentResizeHandle({ width, onWidthChange }: { width: number; onWidthChange: (width: number) => void }) {
  const { t } = useTranslation();
  const [dragging, setDragging] = useState(false);

  useEffect(() => {
    document.body.classList.toggle("agent-resizing", dragging);
    return () => document.body.classList.remove("agent-resizing");
  }, [dragging]);

  return (
    // biome-ignore lint/a11y/useSemanticElements: a focusable window splitter (separator with a value and arrow keys); <hr> cannot take focus or a value
    <div
      aria-label={t("workspace.resizeAgent")}
      aria-orientation="vertical"
      aria-valuemax={clampAgentWidth(MAX_AGENT_WIDTH, window.innerWidth)}
      aria-valuemin={MIN_AGENT_WIDTH}
      aria-valuenow={width}
      className={`ide-agent-resizer${dragging ? " dragging" : ""}`}
      onDoubleClick={() => onWidthChange(DEFAULT_AGENT_WIDTH)}
      onKeyDown={(event) => {
        const step = event.shiftKey ? AGENT_RESIZE_STEP * 4 : AGENT_RESIZE_STEP;
        let next: number | null = null;
        if (event.key === "ArrowLeft") next = width + step;
        else if (event.key === "ArrowRight") next = width - step;
        else if (event.key === "Enter") next = DEFAULT_AGENT_WIDTH;
        if (next === null) return;
        event.preventDefault();
        onWidthChange(clampAgentWidth(next, window.innerWidth));
      }}
      onPointerDown={(event) => {
        event.currentTarget.setPointerCapture(event.pointerId);
        setDragging(true);
      }}
      onPointerMove={(event) => {
        if (dragging) onWidthChange(clampAgentWidth(window.innerWidth - event.clientX, window.innerWidth));
      }}
      onPointerUp={(event) => {
        event.currentTarget.releasePointerCapture(event.pointerId);
        setDragging(false);
      }}
      role="separator"
      tabIndex={0}
    />
  );
}

/**
 * One open version in the editor's tab strip. It is a pair of buttons (open, close), not an ARIA tab: a `tab`
 * cannot hold other controls (its children are presentational, so the close button would vanish for screen
 * readers), and the strip has no arrow-key navigation or tab panels. The open version is marked `aria-current`.
 */
export function EditorTab({
  id,
  roleLabel,
  active,
  unsaved,
  onOpen,
  onClose,
}: {
  id: string;
  roleLabel: string;
  active: boolean;
  unsaved: boolean;
  onOpen: () => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="ide-tab" data-active={active}>
      <button
        aria-current={active ? "true" : undefined}
        className="flex items-center gap-1.5"
        onClick={onOpen}
        type="button"
      >
        <IconPhoto size={13} />
        {id}
        <span className="text-(--ide-text-faint)">{roleLabel}</span>
      </button>
      {unsaved ? (
        <span aria-label={t("workspace.unsaved")} className="ide-tab-dirty" role="img" title={t("workspace.unsaved")} />
      ) : null}
      <button
        aria-label={t("workspace.closeTabNamed", { name: id })}
        className="ide-tab-close"
        onClick={onClose}
        type="button"
      >
        <IconX size={12} />
      </button>
    </div>
  );
}

/**
 * Full-window preview of an image opened from the chat or the explorer. A click anywhere or Escape closes it;
 * focus moves into it on open and back to the opening thumbnail on close.
 */
export function ImageLightbox({ url, onClose }: { url: string; onClose: () => void }) {
  const { t } = useTranslation();
  const ref = useRef<HTMLDivElement | null>(null);
  useModalFocus(ref);

  return (
    <div
      aria-label={t("workspace.imagePreview")}
      aria-modal="true"
      className="ide-lightbox"
      onClick={onClose}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          onClose();
        }
      }}
      ref={ref}
      role="dialog"
      tabIndex={-1}
    >
      <img alt="" src={url} />
    </div>
  );
}
