## 2024-07-24 - Icon-only buttons and loading states
**Learning:** Found a recurring pattern where icon-only action buttons (like Delete) use `title` but lack `aria-label` and `focus-visible` states, making them difficult to navigate via keyboard and less accessible to screen readers. Also, mutation buttons like 'Create' or 'Delete' often just use `disabled` states without visual feedback (loading spinners).
**Action:** Always add `aria-label` with context (e.g. "Delete key [name]") and `focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring` to raw `<button>` elements. Use inline `<Loader2 className="mr-2 h-4 w-4 animate-spin" />` in submit buttons to provide immediate visual feedback.
## 2026-07-25 - Missing focus rings on generic inputs
**Learning:** Found that custom/generic input implementations like `SearchInput` in the app can be missing the standard Shadcn/Tailwind focus rings (`focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none`) that are present on standard `Input` components, leading to broken keyboard navigation for primary page actions.
**Action:** Always verify keyboard accessibility (`focus-visible`) and semantic HTML tags (`type="search"`) on custom input wrappers and icon-only buttons across the application.

I am learning that Linear tickets are no longer used for PR titles.
## 2026-07-29 - [Global refresh visual indicator]
**Learning:** Relying purely on a manual manual loading state in global headers can mismatch UI states if a user triggers an implicit background fetch while also hitting the 'Refresh' button. We need to tie top-level refresh visual indicators to the global `useIsFetching` state.
**Action:** Next time, always check if manual refresh indicators can be augmented with the data library's global fetching hooks (like TanStack's `useIsFetching()`) to represent all ongoing network states.
