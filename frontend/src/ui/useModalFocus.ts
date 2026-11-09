import { type RefObject, useEffect, useState } from "react";

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * Keyboard focus for an overlay that blocks the page (dialogs, the image preview).
 *
 * On mount, focus moves to the element marked `data-autofocus` inside the container, or to the container itself,
 * so Escape and Tab work right away. Tab and Shift+Tab wrap inside the container. On unmount, focus returns to the
 * element that had it before (the button that opened the overlay) if it is still on the page.
 *
 * The container needs `tabIndex={-1}` so it can take focus. `data-autofocus` is used instead of React's
 * `autoFocus` because StrictMode runs this effect twice: the cleanup in between would move focus back out.
 */
export function useModalFocus(containerRef: RefObject<HTMLElement | null>) {
  const [previous] = useState(() => (document.activeElement instanceof HTMLElement ? document.activeElement : null));

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const initial = container.querySelector<HTMLElement>("[data-autofocus]") ?? container;
    initial.focus();

    function trapTab(event: KeyboardEvent) {
      if (event.key !== "Tab" || !container) return;
      const focusable = [...container.querySelectorAll<HTMLElement>(FOCUSABLE)];
      const first = focusable[0];
      const last = focusable.at(-1);
      if (!first || !last) {
        event.preventDefault();
        return;
      }
      const active = document.activeElement;
      if (event.shiftKey && (active === first || active === container)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        first.focus();
      }
    }

    container.addEventListener("keydown", trapTab);
    return () => {
      container.removeEventListener("keydown", trapTab);
      if (previous?.isConnected) previous.focus();
    };
  }, [containerRef, previous]);
}
