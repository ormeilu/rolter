import * as React from "react";

/**
 * What an inline detail drawer owes the keyboard: focus moves into it when it
 * opens (so the next Tab reads its content rather than the row after the one
 * just clicked), Escape closes it, and focus returns to whatever opened it.
 *
 * Deliberately not a modal — the table beside it stays usable — so there is no
 * trap and no scrim; that is `useModalA11y`'s job for dialogs and sheets.
 */
export function useDrawerA11y(open: boolean, onClose: () => void) {
  const ref = React.useRef<HTMLElement | null>(null);
  const onCloseRef = React.useRef(onClose);
  onCloseRef.current = onClose;

  React.useEffect(() => {
    if (!open) return;
    const panel = ref.current;
    if (!panel) return;
    const opener = document.activeElement as HTMLElement | null;
    // a macrotask: the drawer may still be laying out on the frame it mounts
    const timer = setTimeout(() => panel.focus({ preventScroll: true }), 0);
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      // a modal raised over the drawer owns Escape
      if (document.querySelector('[aria-modal="true"]')) return;
      onCloseRef.current();
    };
    document.addEventListener("keydown", onKey);
    return () => {
      clearTimeout(timer);
      document.removeEventListener("keydown", onKey);
      if (opener && opener.isConnected) opener.focus({ preventScroll: true });
    };
  }, [open]);

  return { ref, tabIndex: -1 } as const;
}
