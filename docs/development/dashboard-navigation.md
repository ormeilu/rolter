# Dashboard navigation rail

The left rail (`ui/src/components/ui/nav-sidebar.tsx`) is the dashboard's
primary navigation. It has two independent size controls.

## Collapse

`collapsible` puts a toggle in the brand row that folds the rail down to a
52px icon-only strip. Labels, the search box, group headings and the version
become titles or disappear; the active item keeps its folk-red thread.

## Resize

`resizable` turns the right edge into a splitter. It exists because a fixed
width cannot suit every locale: the `ru` catalog's labels are consistently
longer than `en`'s, and at the shipped 232px several of them truncate with no
way to read the whole label (#950).

- **Bounds** — `NAV_MIN_WIDTH` (180px) to `NAV_MAX_WIDTH` (420px), exported
  from the component. Every path clamps: drag, keyboard, and the value read
  back from storage, so a stale or hand-edited entry cannot restore a rail too
  narrow to click or wide enough to bury the content.
- **Mouse** — drag the edge. Double-click resets to `NAV_DEFAULT_WIDTH`
  (232px, the value `--sidebar-width` carries in `index.css`).
- **Keyboard** — the splitter is a focusable `role="separator"` with
  `aria-orientation="vertical"` and live `aria-valuenow`/`valuemin`/`valuemax`.
  `←`/`→` move it 16px, `Shift` multiplies that by four, `Home` and `End` jump
  to the bounds, and `Enter`/`Space` return it to the default. A mouse-only
  affordance would not be acceptable on a primary nav control.
- **Persistence** — the settled width is written to `localStorage` under
  `storageKey` (`rolter.nav.width` by default), so it survives a reload per
  browser. Reads and writes are both wrapped: a browser that refuses storage
  still resizes, it just forgets. The stored value is applied after mount
  rather than in the state initializer, so the first paint is the same
  everywhere.
- **Collapse wins.** While collapsed there is no splitter at all — a 52px icon
  rail has no edge worth dragging, and leaving it behind would let a keyboard
  user stretch a rail whose labels are hidden.

The width transition is disabled for the duration of a drag; animating it
would fight the pointer.

Stories: `Resizable`, `DraggedNarrow`, `DraggedWide` and
`CollapsedHasNoSplitter` in `nav-sidebar.stories.tsx` cover the bounds, the
keyboard path, the ARIA contract and the collapsed case.
