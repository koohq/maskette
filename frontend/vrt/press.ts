import type { Locator, Page } from "@playwright/test";

type Layout = Record<string, string>;

/**
 * The box of every element on the page but `button` and what is inside it (tooltips left out), and for every box
 * that scrolls, the directions it has something to scroll in: there it shows a scrollbar, which takes room from what
 * is inside. The test browser hides scrollbars, so the room is not taken there and only the second shows it.
 */
function readLayout(button: Locator): Promise<Layout> {
  return button.evaluate((self) => {
    const layout: Record<string, string> = {};
    let next = document.querySelectorAll("[data-press-id]").length;
    for (const element of Array.from(document.querySelectorAll<HTMLElement>("body *"))) {
      // Elements added since the first reading (a tooltip, the press ripple) have no id and are not compared.
      if (!element.hasAttribute("data-press-id")) element.setAttribute("data-press-id", String(next++));
      if (self.contains(element) || element.closest("[role=tooltip], .MuiTooltip-popper")) continue;
      const name =
        element.getAttribute("aria-label") ?? element.getAttribute("data-testid") ?? element.tagName.toLowerCase();
      const key = `${element.getAttribute("data-press-id")} ${name}`;
      const box = element.getBoundingClientRect();
      if (box.width > 0 || box.height > 0) layout[key] = `${box.x},${box.y} ${box.width}×${box.height}`;
      const style = getComputedStyle(element);
      const scrolls = (overflow: string) => overflow === "auto" || overflow === "scroll";
      if (scrolls(style.overflowX) || scrolls(style.overflowY)) {
        const x = scrolls(style.overflowX) && element.scrollWidth > element.clientWidth;
        const y = scrolls(style.overflowY) && element.scrollHeight > element.clientHeight;
        layout[`${key} scrollbar`] = [x ? "across" : "", y ? "down" : ""].filter(Boolean).join("+") || "none";
      }
    }
    return layout;
  });
}

/**
 * Hovers and presses `button` without clicking it (the mouse leaves before it is let go) and lists what changed
 * around it meanwhile. A button rises on hover and sinks while pressed; nothing else should move or start to scroll.
 */
export async function movedWhilePressed(page: Page, button: Locator): Promise<string[]> {
  const before = await readLayout(button);
  const box = await button.boundingBox();
  if (!box) throw new Error("the button is not shown");
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps: 2 });
  // Pointed at for a moment first, as a person does.
  await page.waitForTimeout(400);
  await page.mouse.down();
  // Past the 120 ms the button takes to sink.
  await page.waitForTimeout(300);
  const pressed = await readLayout(button);
  await page.mouse.move(0, 0, { steps: 2 });
  await page.mouse.up();
  // Back in place before the next button is read.
  await page.waitForTimeout(300);
  return Object.keys(before)
    .filter((key) => key in pressed && pressed[key] !== before[key])
    .map((key) => `${key}: ${before[key]} -> ${pressed[key]}`);
}
