import type { Page } from "@playwright/test";

type Shift = { at: number; moved: string[] };

/**
 * Records every layout shift from the page's start, those right after a click included (the CLS score leaves them
 * out, yet they are seen all the same). Call before the page opens.
 */
export async function recordLayoutShifts(page: Page) {
  await page.addInitScript(() => {
    const shifts: Shift[] = [];
    (window as unknown as { layoutShifts: Shift[] }).layoutShifts = shifts;
    const name = (node: Node | null) => {
      if (!(node instanceof Element)) return node?.nodeName ?? "?";
      const className = typeof node.className === "string" ? node.className.split(/\s+/)[0] : "";
      return `${node.tagName.toLowerCase()}${className ? `.${className}` : ""}`;
    };
    const box = (rect: DOMRectReadOnly) => `${rect.x},${rect.y} ${rect.width}×${rect.height}`;
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries() as unknown as {
        startTime: number;
        sources: { node: Node | null; previousRect: DOMRectReadOnly; currentRect: DOMRectReadOnly }[];
      }[]) {
        shifts.push({
          at: Math.round(entry.startTime),
          moved: entry.sources.map(
            (source) => `${name(source.node)}: ${box(source.previousRect)} -> ${box(source.currentRect)}`,
          ),
        });
      }
    }).observe({ type: "layout-shift", buffered: true });
  });
}

/** The shifts recorded so far, once the frames being drawn are reported. */
export async function layoutShifts(page: Page): Promise<Shift[]> {
  await page.evaluate(
    () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(resolve, 50)))),
  );
  return page.evaluate(() => (window as unknown as { layoutShifts: Shift[] }).layoutShifts);
}
