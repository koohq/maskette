import type { Page } from "@playwright/test";

export type OverflowFinding = {
  /**
   * `spills`: drawn outside its box. `clipped`: cut off by an `overflow: hidden` box or an input field.
   * `truncated`: cut off with "…" on purpose. `offscreen`: reaches past the side of the window.
   */
  kind: "spills" | "clipped" | "truncated" | "offscreen";
  /** The text, or the control's name. */
  what: string;
  box: string;
  /** How far it reaches past the box, in CSS pixels (largest side). */
  by: number;
};

/**
 * Finds text and controls that do not fit in the boxes around them. For every visible text node and every control
 * (button, link, field), walks up the boxes that contain it and reports the first one it reaches past; a scrolling
 * box ends the walk, since content there is meant to scroll out of view. Text typed or shown as a placeholder in a
 * one-line field is measured against the field. Runs in the page, so it must not use anything from module scope.
 */
function findOverflowInPage(tolerance: number): OverflowFinding[] {
  const findings: OverflowFinding[] = [];
  const seen = new Set<string>();
  const CONTROLS = "button, a[href], input, select, textarea, [role=button], [role=tab], [role=menuitem]";

  function add(finding: OverflowFinding) {
    const key = `${finding.kind}|${finding.what}|${finding.box}`;
    if (seen.has(key)) return;
    seen.add(key);
    findings.push(finding);
  }

  function boxPath(element: Element): string {
    const parts: string[] = [];
    let current: Element | null = element;
    while (current && current !== document.body && parts.length < 4) {
      let part = current.tagName.toLowerCase();
      const label = current.getAttribute("data-testid") ?? current.getAttribute("aria-label");
      if (label) part += `[${label}]`;
      const classes = [...current.classList].filter((name) => !name.includes(":") && !name.includes("[")).slice(0, 3);
      if (classes.length > 0) part += `.${classes.join(".")}`;
      parts.unshift(part);
      current = current.parentElement;
    }
    return parts.join(" > ");
  }

  function hiddenOnPurpose(element: Element): boolean {
    for (let current: Element | null = element; current; current = current.parentElement) {
      const style = getComputedStyle(current);
      if (style.display === "none" || style.visibility === "hidden" || Number(style.opacity) === 0) return true;
      if (style.clip !== "auto" && style.clip !== "") return true;
      if (style.clipPath !== "none") return true;
      if (current.getAttribute("aria-hidden") === "true") return true;
    }
    return false;
  }

  function nameOf(control: Element): string {
    return (control.getAttribute("aria-label") ?? control.textContent ?? "").trim().slice(0, 60) || control.tagName;
  }

  /** Walks up from `start` and reports the first box the rectangles reach past. */
  function check(what: string, rects: DOMRect[], start: Element | null) {
    for (let box = start; box && box !== document.documentElement; box = box.parentElement) {
      const style = getComputedStyle(box);
      if (style.display === "inline" || style.display === "contents") continue;
      const outer = box.getBoundingClientRect();
      if (outer.width === 0 || outer.height === 0) continue;
      const scrollsX = /auto|scroll/.test(style.overflowX);
      const scrollsY = /auto|scroll/.test(style.overflowY);
      // The padding box: content may run into the padding, but not past the border.
      const left = outer.left + box.clientLeft;
      const top = outer.top + box.clientTop;
      const right = left + box.clientWidth;
      const bottom = top + box.clientHeight;
      let by = 0;
      for (const rect of rects) {
        if (!scrollsX) by = Math.max(by, left - rect.left, rect.right - right);
        if (!scrollsY) by = Math.max(by, top - rect.top, rect.bottom - bottom);
      }
      if (by > tolerance) {
        const clips = style.overflowX !== "visible" || style.overflowY !== "visible";
        const kind = style.textOverflow === "ellipsis" ? "truncated" : clips ? "clipped" : "spills";
        add({ kind, what, box: boxPath(box), by: Math.round(by) });
        return;
      }
      if (scrollsX && scrollsY) return;
    }
    // Nothing contains it, but the window still ends at its sides.
    const by = Math.max(0, ...rects.map((rect) => Math.max(-rect.left, rect.right - window.innerWidth)));
    if (by > tolerance) add({ kind: "offscreen", what, box: "window", by: Math.round(by) });
  }

  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = node.textContent?.trim() ?? "";
    const parent = node.parentElement;
    if (!text || !parent || hiddenOnPurpose(parent)) continue;
    const range = document.createRange();
    range.selectNodeContents(node);
    const rects = [...range.getClientRects()].filter((rect) => rect.width > 0 && rect.height > 0);
    if (rects.length > 0) check(text.slice(0, 60), rects, parent);
  }

  for (const control of document.querySelectorAll(CONTROLS)) {
    if (hiddenOnPurpose(control)) continue;
    const rect = control.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) check(nameOf(control), [rect], control.parentElement);
  }

  const canvas = document.createElement("canvas").getContext("2d");
  for (const field of document.querySelectorAll("input")) {
    if (!canvas || hiddenOnPurpose(field) || !/^(text|search|url|email|number|)$/.test(field.type)) continue;
    const text = field.value || field.placeholder;
    if (!text) continue;
    const style = getComputedStyle(field);
    canvas.font = `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
    const room = field.clientWidth - Number.parseFloat(style.paddingLeft) - Number.parseFloat(style.paddingRight);
    const by = canvas.measureText(text).width - room;
    // A URL or key is expected to scroll inside a long field; only a field too narrow for short text is reported.
    if (by > tolerance && room < 400) {
      add({ kind: "clipped", what: text.slice(0, 60), box: boxPath(field), by: Math.round(by) });
    }
  }
  return findings;
}

/** Text and controls on the page that do not fit in their boxes (see `findOverflowInPage`). */
export function findOverflow(page: Page, tolerance = 1): Promise<OverflowFinding[]> {
  return page.evaluate(findOverflowInPage, tolerance);
}
