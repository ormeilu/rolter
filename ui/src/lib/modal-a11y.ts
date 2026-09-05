import * as React from "react";

// what a modal owes the keyboard and the screen reader, shared by Dialog and
// Sheet so neither can claim `aria-modal` without honouring it (#1181):
//
// - focus moves into the panel when it opens and back to the opener when it
//   closes, so a keyboard user is never left on an element behind the scrim
// - Tab and Shift+Tab cycle inside the panel instead of walking into the page
// - Escape closes only the topmost modal, so a dialog raised over a sheet does
//   not take the sheet down with it
// - the page behind stops scrolling while a modal is up

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

const FORM_CONTROL =
  'input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled])';

// open modals, innermost last. module-level on purpose: the stack is a fact
// about the document, not about any one react tree
const stack: symbol[] = [];

function focusables(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    (el) => el.offsetParent !== null || el === document.activeElement,
  );
}

export interface ModalA11yOptions {
  open: boolean;
  /** invoked for Escape; the modal decides whether that actually closes it */
  onEscape: () => void;
  /**
   * where focus lands on open. `"first"` picks the first focusable control,
   * which suits a form; `"panel"` focuses the container itself, which suits a
   * confirmation whose first control is a destructive button
   */
  initialFocus?: "first" | "panel";
}

/**
 * Wire the modal behaviours above onto the element in `ref`.
 *
 * The returned props go on the panel: it needs `tabIndex={-1}` so it can
 * receive focus when it has no controls, and the key handler that traps Tab.
 */
export function useModalA11y(
  ref: React.RefObject<HTMLElement | null>,
  { open, onEscape, initialFocus = "first" }: ModalA11yOptions,
) {
  const id = React.useRef<symbol | null>(null);
  const onEscapeRef = React.useRef(onEscape);
  onEscapeRef.current = onEscape;

  React.useEffect(() => {
    if (!open) return;
    const panel = ref.current;
    if (!panel) return;
    const token = Symbol("modal");
    id.current = token;
    stack.push(token);

    const opener = document.activeElement as HTMLElement | null;
    const prevOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";

    // focus after paint so the enter animation has laid the panel out. a
    // pointer user may already have clicked into a field by then; their focus
    // wins. "first" prefers a form control over the close button that opens
    // every header, and falls back to the panel when there is none
    // a macrotask rather than requestAnimationFrame: a hidden tab pauses
    // animation frames, and a modal raised there still needs its focus set
    const frame = setTimeout(() => {
      if (panel.contains(document.activeElement)) return;
      const target =
        initialFocus === "first"
          ? (panel.querySelector<HTMLElement>(FORM_CONTROL) ?? focusables(panel)[0])
          : undefined;
      (target ?? panel).focus({ preventScroll: true });
    }, 0);

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (stack[stack.length - 1] !== token) return;
      e.stopPropagation();
      onEscapeRef.current();
    };
    document.addEventListener("keydown", onKeyDown);

    return () => {
      clearTimeout(frame);
      document.removeEventListener("keydown", onKeyDown);
      const at = stack.indexOf(token);
      if (at >= 0) stack.splice(at, 1);
      if (stack.length === 0) document.body.style.overflow = prevOverflow;
      // the opener may have unmounted with the row it sat in; then there is
      // nothing sensible to return to and the browser's default is fine
      if (opener && opener.isConnected) opener.focus({ preventScroll: true });
    };
  }, [open, ref, initialFocus]);

  const onKeyDown = React.useCallback(
    (e: React.KeyboardEvent<HTMLElement>) => {
      if (e.key !== "Tab") return;
      const panel = ref.current;
      if (!panel) return;
      const items = focusables(panel);
      if (items.length === 0) {
        e.preventDefault();
        panel.focus();
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
      const active = document.activeElement;
      if (e.shiftKey && (active === first || active === panel)) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && active === last) {
        e.preventDefault();
        first.focus();
      }
    },
    [ref],
  );

  return { tabIndex: -1, onKeyDown } as const;
}

/** how many modals are open — tests and the nav's outside-click guard use it */
export function openModalCount(): number {
  return stack.length;
}
