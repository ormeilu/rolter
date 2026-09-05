import * as React from "react";

// one-shot feedback for the whole dashboard (#1197): a save that went through,
// a delete that did not. inline messages stay for field-level validation,
// which belongs next to the field; anything that would otherwise vanish when
// a sheet closes goes here instead
export type ToastTone = "success" | "error" | "info";

export interface Toast {
  id: number;
  tone: ToastTone;
  title: string;
  /** optional second line — the control plane's own message on a failure */
  detail?: string;
}

export interface ToastInput {
  tone?: ToastTone;
  title: string;
  detail?: string;
  /** milliseconds before auto-dismiss; errors default to staying longer */
  duration?: number;
}

interface ToastApi {
  toasts: Toast[];
  push: (input: ToastInput) => number;
  dismiss: (id: number) => void;
}

const ToastContext = React.createContext<ToastApi | null>(null);

// dismiss timings: a success is glanced at, a failure has to be read
const SUCCESS_MS = 4000;
const ERROR_MS = 8000;
// how many stay on screen at once; older ones drop off first
const MAX_VISIBLE = 4;

export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [toasts, setToasts] = React.useState<Toast[]>([]);
  const counter = React.useRef(0);
  const timers = React.useRef(new Map<number, ReturnType<typeof setTimeout>>());

  const dismiss = React.useCallback((id: number) => {
    const timer = timers.current.get(id);
    if (timer) clearTimeout(timer);
    timers.current.delete(id);
    setToasts((all) => all.filter((t) => t.id !== id));
  }, []);

  const push = React.useCallback(
    ({ tone = "info", title, detail, duration }: ToastInput) => {
      const id = ++counter.current;
      setToasts((all) => [...all, { id, tone, title, detail }].slice(-MAX_VISIBLE));
      const ms = duration ?? (tone === "error" ? ERROR_MS : SUCCESS_MS);
      timers.current.set(
        id,
        setTimeout(() => dismiss(id), ms),
      );
      return id;
    },
    [dismiss],
  );

  React.useEffect(() => {
    const pending = timers.current;
    return () => pending.forEach((timer) => clearTimeout(timer));
  }, []);

  const api = React.useMemo(() => ({ toasts, push, dismiss }), [toasts, push, dismiss]);
  return <ToastContext.Provider value={api}>{children}</ToastContext.Provider>;
}

const NOOP: ToastApi = { toasts: [], push: () => 0, dismiss: () => {} };

/**
 * The toast queue. Outside a provider — a story rendered on its own, a unit
 * test — it is a no-op rather than a thrown error, so a screen never has to
 * know whether the shell is around it.
 */
export function useToast(): ToastApi {
  return React.useContext(ToastContext) ?? NOOP;
}

/** the message an `ApiError` or a thrown value carries, for a toast's detail */
export function errorDetail(error: unknown): string | undefined {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return undefined;
}
