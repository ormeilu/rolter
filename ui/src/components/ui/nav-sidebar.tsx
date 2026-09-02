import { ChevronDown, ChevronsUpDown, PanelLeftClose, PanelLeftOpen, Search, X } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";

// left rail: brand + collapse toggle, nav search, nav groups (flat items or
// collapsible parents with sub-items), footer links + version, org/user block
// pinned to the bottom. collapses to an icon-only rail. the active item carries
// the folk-red вышивка thread on its left edge. mirrors the Rolter Design
// System navigation/NavSidebar.
export interface NavItem {
  key: string;
  label: string;
  icon?: React.ReactNode;
  count?: number;
  children?: NavItem[];
}

export interface NavGroup {
  label?: string;
  items: NavItem[];
}

export interface NavUser {
  name: React.ReactNode;
  role?: React.ReactNode;
  initials: React.ReactNode;
  onClick?: () => void;
}

export interface NavFooterLink {
  key: string;
  icon: React.ReactNode;
  title: string;
  onClick?: () => void;
  href?: string;
}

export interface NavSidebarProps extends React.HTMLAttributes<HTMLElement> {
  brand?: React.ReactNode;
  logoSrc?: string;
  groups: NavGroup[];
  activeKey?: string;
  onNavigate?: (key: string) => void;
  user?: NavUser;
  /* when provided, the user block becomes a menu trigger: clicking it opens a
     popover above the block rendering this content (scope switcher, account,
     sign out). `close` dismisses the popover. takes precedence over
     user.onClick. */
  userMenu?: (close: () => void) => React.ReactNode;
  /* bifrost-style extras — all optional so existing call sites keep working */
  searchable?: boolean;
  collapsible?: boolean;
  defaultCollapsed?: boolean;
  footerLinks?: NavFooterLink[];
  /* rendered in the footer row after the links and before the version — the
     locale picker sits here, so it stays on screen whatever the active route.
     receives the rail state so it can drop its label when collapsed. */
  footerExtra?: (collapsed: boolean) => React.ReactNode;
  version?: string;
  /* when set, the right edge becomes a draggable, keyboard-operable splitter
     and the rail's width is remembered per browser under `storageKey`. the
     width is clamped to [NAV_MIN_WIDTH, NAV_MAX_WIDTH] on every path — drag,
     keyboard and the value read back from storage — so a stale or hand-edited
     entry cannot restore an unusable rail. */
  resizable?: boolean;
  storageKey?: string;
}

/** narrowest the rail may be dragged: still wide enough for icon + label */
export const NAV_MIN_WIDTH = 180;
/** widest: past this the rail competes with the content it navigates */
export const NAV_MAX_WIDTH = 420;
/** matches `--sidebar-width` in index.css */
export const NAV_DEFAULT_WIDTH = 232;
const NAV_WIDTH_STORAGE_KEY = "rolter.nav.width";
/** one arrow press; shift multiplies it so a keyboard user is not stuck stepping */
const NAV_KEY_STEP = 16;

const clampWidth = (w: number) =>
  Math.min(NAV_MAX_WIDTH, Math.max(NAV_MIN_WIDTH, Math.round(w)));

function readStoredWidth(key: string): number | null {
  // storage throws outright in some embedding contexts, and can hold anything
  try {
    const raw = window.localStorage.getItem(key);
    if (raw === null) return null;
    const n = Number(raw);
    return Number.isFinite(n) ? clampWidth(n) : null;
  } catch {
    return null;
  }
}

const itemBase =
  "relative flex w-full items-center gap-2 rounded-md border-none bg-transparent px-2 py-1.5 text-left text-sm transition-colors [&>svg]:h-4 [&>svg]:w-4 [&>svg]:flex-none focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring";
const itemIdle = "text-muted-foreground hover:bg-muted hover:text-foreground";
const itemActive =
  "bg-[color:var(--surface-subtle)] text-foreground before:absolute before:-left-px before:top-1/2 before:h-4 before:w-[3px] before:-translate-y-1/2 before:rounded-full before:bg-[color:var(--red-folk)] before:content-['']";

function matches(it: NavItem, q: string): boolean {
  if (it.label.toLowerCase().includes(q)) return true;
  return (it.children ?? []).some((c) => matches(c, q));
}

export function NavSidebar({
  brand = "rolter",
  logoSrc,
  groups = [],
  activeKey,
  onNavigate,
  user,
  userMenu,
  searchable,
  collapsible,
  defaultCollapsed,
  footerLinks,
  footerExtra,
  version,
  resizable,
  storageKey = NAV_WIDTH_STORAGE_KEY,
  className,
  ...props
}: NavSidebarProps) {
  const { t } = useTranslation();
  const [collapsed, setCollapsed] = React.useState(defaultCollapsed ?? false);
  const [query, setQuery] = React.useState("");
  // parents stay open once toggled; the one holding the active child opens itself
  const [open, setOpen] = React.useState<Record<string, boolean>>({});
  const [userOpen, setUserOpen] = React.useState(false);
  const userRef = React.useRef<HTMLDivElement>(null);
  const navRef = React.useRef<HTMLElement>(null);
  const [width, setWidth] = React.useState(NAV_DEFAULT_WIDTH);
  const [dragging, setDragging] = React.useState(false);

  // read the remembered width after mount rather than in the initializer: the
  // first paint then matches what a server-rendered or storage-less browser
  // draws, and the story harness gets a deterministic starting width
  React.useEffect(() => {
    if (!resizable) return;
    const stored = readStoredWidth(storageKey);
    if (stored !== null) setWidth(stored);
  }, [resizable, storageKey]);

  const commitWidth = React.useCallback(
    (next: number) => {
      const w = clampWidth(next);
      setWidth(w);
      try {
        window.localStorage.setItem(storageKey, String(w));
      } catch {
        // a browser that refuses storage still resizes, it just forgets
      }
    },
    [storageKey],
  );

  // the pointer leaves the 4px handle immediately on a fast drag, so the move
  // and up listeners live on the document for the duration of the gesture
  React.useEffect(() => {
    if (!dragging) return;
    const onMove = (e: PointerEvent) => {
      const left = navRef.current?.getBoundingClientRect().left ?? 0;
      setWidth(clampWidth(e.clientX - left));
    };
    const onUp = () => setDragging(false);
    document.addEventListener("pointermove", onMove);
    document.addEventListener("pointerup", onUp);
    document.addEventListener("pointercancel", onUp);
    return () => {
      document.removeEventListener("pointermove", onMove);
      document.removeEventListener("pointerup", onUp);
      document.removeEventListener("pointercancel", onUp);
    };
  }, [dragging]);

  // persist once the gesture ends, not on every pointermove
  const wasDragging = React.useRef(false);
  React.useEffect(() => {
    if (wasDragging.current && !dragging) commitWidth(width);
    wasDragging.current = dragging;
  }, [dragging, width, commitWidth]);

  const onHandleKeyDown = (e: React.KeyboardEvent) => {
    const step = e.shiftKey ? NAV_KEY_STEP * 4 : NAV_KEY_STEP;
    if (e.key === "ArrowLeft") commitWidth(width - step);
    else if (e.key === "ArrowRight") commitWidth(width + step);
    else if (e.key === "Home") commitWidth(NAV_MIN_WIDTH);
    else if (e.key === "End") commitWidth(NAV_MAX_WIDTH);
    else if (e.key === "Enter" || e.key === " ") commitWidth(NAV_DEFAULT_WIDTH);
    else return;
    e.preventDefault();
  };

  const showHandle = Boolean(resizable) && !collapsed;

  React.useEffect(() => {
    if (!userOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (userRef.current && !userRef.current.contains(e.target as Node)) setUserOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setUserOpen(false);
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [userOpen]);

  // the search box is hidden while collapsed, so the filter must not apply
  const q = collapsed ? "" : query.trim().toLowerCase();

  const isOpen = (it: NavItem) =>
    q !== "" ||
    (open[it.key] ?? (it.children ?? []).some((c) => c.key === activeKey));

  const renderItem = (it: NavItem, depth: number) => {
    if (q && !matches(it, q)) return null;
    const hasKids = (it.children?.length ?? 0) > 0;
    const active = it.key === activeKey;
    const expanded = hasKids && isOpen(it);
    return (
      <React.Fragment key={it.key}>
        <button
          aria-current={active ? "page" : undefined}
          aria-expanded={hasKids ? expanded : undefined}
          title={collapsed ? it.label : undefined}
          onClick={() =>
            hasKids
              ? setOpen((o) => ({ ...o, [it.key]: !isOpen(it) }))
              : onNavigate?.(it.key)
          }
          className={cn(
            itemBase,
            active ? itemActive : itemIdle,
            collapsed && "justify-center px-0",
          )}
        >
          {it.icon}
          {!collapsed && <span className="min-w-0 truncate">{it.label}</span>}
          {!collapsed && it.count != null && (
            <span className="ml-auto font-mono text-[0.6875rem] text-[color:var(--text-subtle)]">
              {it.count}
            </span>
          )}
          {!collapsed && hasKids && (
            <ChevronDown
              className={cn(
                "!ml-auto !h-3.5 !w-3.5 text-[color:var(--text-subtle)] transition-transform",
                !expanded && "-rotate-90",
              )}
            />
          )}
        </button>
        {expanded && !collapsed && (
          <div className="ml-[15px] flex flex-col gap-0.5 border-l border-[color:var(--border-subtle)] pl-1.5">
            {it.children!.map((c) => renderItem(c, depth + 1))}
          </div>
        )}
      </React.Fragment>
    );
  };

  return (
    <nav
      ref={navRef}
      style={showHandle ? { width } : undefined}
      className={cn(
        "relative flex h-full flex-col gap-3 border-r border-[color:var(--border-subtle)] bg-[color:var(--surface-app)] px-2 py-3",
        // the width transition would fight the pointer during a drag
        !dragging && "transition-[width]",
        collapsed ? "w-[52px] items-stretch" : "w-[var(--sidebar-width)]",
        className,
      )}
      {...props}
    >
      {showHandle && (
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label={t("shell.resizeSidebar")}
          aria-valuenow={width}
          aria-valuemin={NAV_MIN_WIDTH}
          aria-valuemax={NAV_MAX_WIDTH}
          tabIndex={0}
          onPointerDown={(e) => {
            e.preventDefault();
            setDragging(true);
          }}
          onDoubleClick={() => commitWidth(NAV_DEFAULT_WIDTH)}
          onKeyDown={onHandleKeyDown}
          className={cn(
            "absolute inset-y-0 -right-1 z-30 w-2 cursor-col-resize focus-visible:outline-none",
            // a 2px thread on the border line, painted only when the splitter
            // is grabbed or focused so the idle rail stays quiet
            "after:absolute after:inset-y-0 after:left-1/2 after:w-[2px] after:-translate-x-1/2 after:bg-[color:var(--red-folk)] after:opacity-0 after:transition-opacity after:content-['']",
            "hover:after:opacity-60 focus-visible:after:opacity-100",
            dragging && "after:opacity-100",
          )}
        />
      )}
      <div
        className={cn(
          "group relative flex items-center gap-2 px-2 py-1.5",
          collapsed && "justify-center px-0",
        )}
      >
        {logoSrc ? <img className="block h-6 w-6 flex-none" src={logoSrc} alt="" /> : null}
        {!collapsed && (
          <span className="font-mono text-lg font-semibold leading-none tracking-[-0.03em] text-foreground">
            {brand}
            <span className="text-[color:var(--red-folk)]">.</span>
          </span>
        )}
        {collapsible && !collapsed && (
          <button
            title={t("shell.collapseSidebar")}
            aria-label={t("shell.collapseSidebar")}
            onClick={() => setCollapsed(true)}
            className="ml-auto rounded-md p-1 text-[color:var(--text-subtle)] transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          >
            <PanelLeftClose className="h-4 w-4" />
          </button>
        )}
        {/* collapsed: the expand toggle lives inside the logo/header area,
            hidden by default and revealed only on hover or keyboard focus of
            the header container. it overlays the centered logo so the rail
            stays icon-narrow. */}
        {collapsible && collapsed && (
          <button
            title={t("shell.expandSidebar")}
            aria-label={t("shell.expandSidebar")}
            onClick={() => setCollapsed(false)}
            className="absolute inset-0 flex items-center justify-center rounded-md bg-[color:var(--surface-app)] text-[color:var(--text-subtle)] opacity-0 transition-opacity duration-200 hover:bg-muted hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          >
            <PanelLeftOpen className="h-4 w-4" />
          </button>
        )}
      </div>

      {/* the focus ring lives on the wrapper, not the input: the input is
          borderless inside a bordered box, so a ring on it would draw inside
          that box rather than around the control a keyboard user sees (#963) */}
      {searchable && !collapsed && (
        <label className="flex items-center gap-2 rounded-md border border-[color:var(--border-subtle)] bg-[color:var(--surface-base)] px-2 py-1.5 transition-colors focus-within:border-[color:var(--border-default)] focus-within:ring-1 focus-within:ring-ring">
          <Search className="h-3.5 w-3.5 flex-none text-[color:var(--text-subtle)]" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("common.search")}
            aria-label={t("shell.searchNav")}
            className="w-full min-w-0 bg-transparent text-sm text-foreground outline-none placeholder:text-[color:var(--text-subtle)]"
          />
          {query && (
            <button
              onClick={() => setQuery("")}
              title={t("common.clearSearch")}
              aria-label={t("common.clearSearch")}
              className="-my-1 -mr-1 rounded p-1 text-[color:var(--text-subtle)] transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          )}
        </label>
      )}

      <div className="flex flex-1 flex-col gap-4 overflow-y-auto">
        {groups.map((g, gi) => {
          const items = g.items.map((it) => renderItem(it, 0)).filter(Boolean);
          if (q && items.length === 0) return null;
          return (
            <div className="flex flex-col gap-0.5" key={g.label || gi}>
              {g.label && !collapsed && (
                <div className="px-2 py-1.5 text-[0.6875rem] uppercase tracking-[0.08em] text-[color:var(--text-subtle)]">
                  {g.label}
                </div>
              )}
              {items}
            </div>
          );
        })}
      </div>

      {(footerLinks?.length || footerExtra || version) && (
        <div className={cn("flex flex-col gap-1.5", collapsed && "items-center")}>
          <div className={cn("flex items-center gap-1 px-1", collapsed && "flex-col px-0")}>
            {footerLinks?.map((l) =>
              l.href ? (
                <a
                  key={l.key}
                  href={l.href}
                  target="_blank"
                  rel="noreferrer"
                  title={l.title}
                  aria-label={l.title}
                  className="rounded-md p-1.5 text-[color:var(--text-subtle)] transition-colors hover:bg-muted hover:text-foreground [&>svg]:h-4 [&>svg]:w-4 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                >
                  {l.icon}
                </a>
              ) : (
                <button
                  key={l.key}
                  onClick={l.onClick}
                  title={l.title}
                  aria-label={l.title}
                  className="rounded-md p-1.5 text-[color:var(--text-subtle)] transition-colors hover:bg-muted hover:text-foreground [&>svg]:h-4 [&>svg]:w-4 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                >
                  {l.icon}
                </button>
              ),
            )}
            {footerExtra?.(collapsed)}
            {version && !collapsed && (
              <span className="ml-auto pr-1 font-mono text-[0.6875rem] text-[color:var(--text-subtle)]">
                {version}
              </span>
            )}
          </div>
        </div>
      )}

      {user && (
        <div
          ref={userRef}
          className={cn("relative mt-auto flex flex-col gap-1.5", collapsed && "items-center")}
        >
          {userMenu && userOpen && (
            <div
              className={cn(
                "absolute bottom-[calc(100%+6px)] z-40 rounded-lg border border-[color:var(--border-default)] bg-[color:var(--surface-elevated)] py-1.5 shadow-lg",
                collapsed ? "left-0 w-[260px]" : "inset-x-0",
              )}
            >
              {userMenu(() => setUserOpen(false))}
            </div>
          )}
          <button
            onClick={() => (userMenu ? setUserOpen((v) => !v) : user.onClick?.())}
            aria-haspopup={userMenu ? "menu" : undefined}
            aria-expanded={userMenu ? userOpen : undefined}
            title={collapsed && typeof user.name === "string" ? user.name : undefined}
            className={cn(
              "flex items-center gap-2 rounded-md transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
              collapsed
                ? "justify-center p-0.5 hover:bg-muted"
                : "border border-[color:var(--border-subtle)] bg-[color:var(--surface-base)] px-2 py-1.5 hover:border-[color:var(--border-default)]",
              userOpen && !collapsed && "border-[color:var(--border-default)]",
            )}
          >
            <span className="flex h-[26px] w-[26px] flex-none items-center justify-center rounded-full bg-[color:var(--red-folk)] text-xs font-semibold text-white">
              {user.initials}
            </span>
            {!collapsed && (
              <>
                <span className="flex min-w-0 flex-col">
                  <span className="overflow-hidden text-ellipsis whitespace-nowrap text-xs font-medium text-foreground">
                    {user.name}
                  </span>
                  {user.role && (
                    <span className="overflow-hidden text-ellipsis whitespace-nowrap text-[0.6875rem] text-muted-foreground">
                      {user.role}
                    </span>
                  )}
                </span>
                <ChevronsUpDown className="ml-auto h-3.5 w-3.5 text-[color:var(--text-subtle)]" />
              </>
            )}
          </button>
        </div>
      )}
    </nav>
  );
}
