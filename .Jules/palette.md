## 2024-08-09 - Added Loading States to Connector Actions
**Learning:** For list items mapping over a shared `useMutation`, using `isPending` without checking `variables` disables the action across all list items globally, which is confusing UX.
**Action:** When working with row-level actions in a mapped list, always qualify loading/disabled states using `mutation.variables === row.id` to localize the pending visual state to just the active row. Ensure buttons provide visual feedback (e.g., `<Loader2 className="animate-spin" />`) during async operations and add `disabled:opacity-50 disabled:pointer-events-none` to prevent duplicate submissions.
## 2026-08-10 - [Keyboard Accessibility: Focus Rings on Buttons]
**Learning:** Many custom-styled icon buttons and list-item buttons lacked visible focus states for keyboard navigation, making the interface harder to use for accessibility. We applied consistent `focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring` Tailwind classes across multiple pages.
**Action:** Ensure all future custom interactive elements explicitly handle keyboard focus states using existing Tailwind ring utilities.
## 2024-03-24 - [Add Loading State to Mapped Items Destructive Actions]
**Learning:** Destructive row actions that use `useMutation` can't rely just on `isPending` without checking `variables` since the mutation state is shared across all list items being mapped over.
**Action:** When mapping over items with a shared mutation, qualify the loading state using `mutation.isPending && mutation.variables === row.id`.
