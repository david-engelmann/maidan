// The one dialog focus contract: focus moves in, Tab and Shift+Tab stay in,
// Escape closes, and focus goes back to whatever opened the dialog.
import { type RefObject, useEffect, useRef } from "react";

export const FOCUSABLE = "button:not([disabled]), a[href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex='0']";

/**
 * Where Tab goes from `at` (an index into the dialog's focusables, or -1 when
 * focus is outside them), or null to let the browser move it.
 */
export function trapTarget(count: number, at: number, shift: boolean): number | null {
  if (count <= 0) return null;
  if (at < 0) return shift ? count - 1 : 0;
  if (shift && at === 0) return count - 1;
  if (!shift && at === count - 1) return 0;
  return null;
}

export function useDialogFocus(box: RefObject<HTMLElement | null>, onClose: () => void, opts: { autofocus?: string; active?: boolean } = {}): void {
  const close = useRef(onClose);
  useEffect(() => {
    close.current = onClose;
  });
  const active = opts.active ?? true;
  const autofocus = opts.autofocus;
  useEffect(() => {
    if (!active) return;
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const el = box.current;
    const first = (autofocus ? el?.querySelector<HTMLElement>(autofocus) : null) ?? el?.querySelector<HTMLElement>(FOCUSABLE) ?? el;
    first?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        close.current();
        return;
      }
      if (e.key !== "Tab" || !box.current) return;
      const f = Array.from(box.current.querySelectorAll<HTMLElement>(FOCUSABLE));
      const to = trapTarget(f.length, f.indexOf(document.activeElement as HTMLElement), e.shiftKey);
      if (to === null) return;
      e.preventDefault();
      f[to]?.focus();
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      if (opener?.isConnected) opener.focus();
    };
  }, [box, active, autofocus]);
}
