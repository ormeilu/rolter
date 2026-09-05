# Dashboard theme

## The dashboard is dark-only

There is one palette and no theme toggle. `ui/src/index.css` is that palette:
`:root` holds the shadcn HSL contract, the zinc ramp, the folkloric red accent,
the status hues, the surfaces and the type scale, and `:root` also declares
`color-scheme: dark` so the browser paints scrollbars and form controls to
match. There is no light theme to fall back to and no plan for one.

Because of that, **`dark:` variants are banned**. Tailwind's `darkMode` option
is not set, so `dark:bg-…` and `dark:text-…` compile to nothing at all — a
class that looks like a decision but ships as dead weight. Write the one value
you mean:

```tsx
// no
<p className="text-amber-600 dark:text-amber-500">…</p>
// yes
<p className="text-[color:var(--status-warning-text)]">…</p>
```

## Colour comes from the tokens

Never hard-code a hex, and never reach for a raw Tailwind palette colour
(`text-emerald-600`, `bg-blue-500/15`, `text-red-400`). Those bypass the design
system: they are not retunable, they are not contrast-checked against the
rolter surface, and they drift out of family with everything around them.
Reference the token instead — `text-[color:var(--status-danger)]`,
`bg-[color:var(--red-tint)]`, `border-[color:var(--border-subtle)]`.

## Status colours come in two flavours

Each status hue ships as a pair, and picking the wrong half is a contrast bug:

| Token | Use for |
|---|---|
| `--status-success` / `--status-warning` / `--status-info` / `--status-danger` | fills: dots, bars, chart series, `/15` tints, borders |
| `--status-success-text` / `--status-warning-text` / `--status-info-text` / `--status-danger-text` | text: badge labels, inline warnings, delta figures |

The fill hues are tuned to carry a shape. As *text* on `--surface-base`
(`#111113`) they are marginal or fail outright — `--status-info` is 4.34:1 on
its own `/15` tint and `--status-danger` is 3.93:1, both under the 4.5:1 WCAG AA
floor for body text. Each `-text` token is the same OKLCH hue and chroma with
the lightness lifted until it clears 4.5:1 on both `#111113` and its own tint,
with headroom (success 5.48:1 on tint, warning 6.84:1, info 5.53:1, danger
5.55:1). `--status-warning-text` equals `--status-warning`, because the amber
already cleared; it exists so a component author never has to know which hue
happens to pass.

The `Badge` component in `ui/src/components/ui/badge.tsx` is the reference
implementation: `/15` tint off the fill hue, label off the matching `-text`
token. Its `AllTones` story asserts the computed label colour still resolves to
the token, so a tone that drifts back to a raw palette colour fails the story
tests.

When you add a status hue, add both halves, check the ratio against `#111113`
*and* against the tint the text will sit on, and record the numbers in the
comment beside the token.
