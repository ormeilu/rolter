## 2024-07-24 - Icon-only buttons and loading states
**Learning:** Found a recurring pattern where icon-only action buttons (like Delete) use `title` but lack `aria-label` and `focus-visible` states, making them difficult to navigate via keyboard and less accessible to screen readers. Also, mutation buttons like 'Create' or 'Delete' often just use `disabled` states without visual feedback (loading spinners).
**Action:** Always add `aria-label` with context (e.g. "Delete key [name]") and `focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring` to raw `<button>` elements. Use inline `<Loader2 className="mr-2 h-4 w-4 animate-spin" />` in submit buttons to provide immediate visual feedback.
## 2026-07-25 - Missing focus rings on generic inputs
**Learning:** Found that custom/generic input implementations like `SearchInput` in the app can be missing the standard Shadcn/Tailwind focus rings (`focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none`) that are present on standard `Input` components, leading to broken keyboard navigation for primary page actions.
**Action:** Always verify keyboard accessibility (`focus-visible`) and semantic HTML tags (`type="search"`) on custom input wrappers and icon-only buttons across the application.

I am learning that Linear tickets are no longer used for PR titles.

## 2024-05-18 - [Missing a11y & async feedback on Row Actions]
**Learning:** Found a recurring pattern in data tables where icon-only action buttons (like Delete) in rows lack `aria-label` attributes and keyboard focus states (`focus-visible` classes). Additionally, confirmation dialogs for these destructive row actions often lack visual loading indicators (`Loader2` from lucide-react) while the asynchronous mutation is pending. This makes keyboard navigation difficult and leaves users wondering if their delete request was registered.
**Action:** When working on data table rows or generic item listings, explicitly verify that all icon-only buttons include an `aria-label` and `focus-visible` classes. Also, always ensure the corresponding confirmation dialogs provide visual loading feedback via `Loader2` during the mutation.
