# Dashboard error states

A screen that cannot load its data has to say *why*. Before #962 every screen
rendered the same sentence — `Failed to load X.` — for causes needing entirely
different responses, so it pointed at none of them.

That is not a hypothetical cost. During the #924 dogfooding pass the Keys screen
showed "Failed to load your keys." while the real cause was every
`/api/v1/me/*` route returning 401 (#942). The message sent the operator to
check their key configuration; the actual cause was found afterwards by reading
traces. An error that cannot separate *you are not signed in* from *the server
is down* costs more time than no error at all, because it invites a wrong
hypothesis and the operator spends their attention there first.

## The rule

Never render a load failure by hand. Use `LoadError`:

```tsx
{keys.error && (
  <LoadError
    error={keys.error}
    resource={t("errors.resources.virtualKeys")}
    onRetry={() => keys.refetch()}
  />
)}
```

`resource` is the translated noun for what failed — it is interpolated into the
title, so it reads as a sentence in every locale. Pass `onRetry` whenever the
caller holds a query handle; the component decides whether offering it is
honest.

## What it distinguishes

`classifyLoadError` in `ui/src/lib/load-error.ts` maps a thrown value to one of
six kinds. `ApiError` already carries `status` and the control plane's `code`,
so no screen has to parse a message to find out what happened.

| kind | cause | recovery offered |
| --- | --- | --- |
| `unauthenticated` | 401 | sign in again |
| `forbidden` | 403 | none — ask an administrator |
| `openMode` | 401 with code `open_mode_no_session` | none — set `ROLTER_ADMIN_TOKEN` |
| `unreachable` | the thrown value is not an `ApiError`, so `fetch` never connected | retry |
| `server` | 5xx | retry |
| `unknown` | any other non-ok status | retry |

Two of these are easy to collapse and must not be. A plain 401 is fixed by
signing in; `open_mode_no_session` is a control plane running with no admin
token, which has no accounts to sign into at all — signing in again is exactly
the wrong advice. And a retry button on a 403 suggests the failure was transient
when it was a permission, so `isRetryable` withholds it.

## Two things that are not this component

**An empty result is not a failure.** A successful request returning zero rows
renders an empty state. Routing it here would tell an operator something is
broken when nothing is.

**The control plane's own message is never swallowed.** `LoadError` prints it
beneath the summary. The dashboard's classification is a helpful gloss, not a
replacement — #962 happened because the gloss was the only thing on screen and
it was wrong.

## Adding a screen

Add the resource noun to `errors.resources.*` in **every** catalog under
`ui/src/lib/i18n/locales/` (see [i18n](i18n.md)) and use it as above. The six
`errors.load.*` kinds already exist; a new screen needs no new error copy.

A *mutation* that fails is a different surface: it is reported where the action
was taken, not where the data would have been. For a destructive action that
means inside the confirmation, which stays open so the message has somewhere to
live — see [destructive actions](destructive-actions.md).
