## 2024-08-09 - Added Loading States to Connector Actions
**Learning:** For list items mapping over a shared `useMutation`, using `isPending` without checking `variables` disables the action across all list items globally, which is confusing UX.
**Action:** When working with row-level actions in a mapped list, always qualify loading/disabled states using `mutation.variables === row.id` to localize the pending visual state to just the active row. Ensure buttons provide visual feedback (e.g., `<Loader2 className="animate-spin" />`) during async operations and add `disabled:opacity-50 disabled:pointer-events-none` to prevent duplicate submissions.
## 2026-08-10 - [Keyboard Accessibility: Focus Rings on Buttons]
**Learning:** Many custom-styled icon buttons and list-item buttons lacked visible focus states for keyboard navigation, making the interface harder to use for accessibility. We applied consistent `focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring` Tailwind classes across multiple pages.
**Action:** Ensure all future custom interactive elements explicitly handle keyboard focus states using existing Tailwind ring utilities.

## 2023-10-27 - Disabled styling on buttons
**Learning:** When adding `disabled={true}` to buttons, they might not inherently visually reflect it correctly without explicit Tailwind styling. `disabled:pointer-events-none disabled:opacity-50` ensures that a disabled button is visually dimmed and doesn't trigger hover events.
**Action:** When updating standard HTML buttons or custom components to respond to `isPending` states, ensure `disabled:pointer-events-none disabled:opacity-50` are added to the Tailwind classes.
## 2023-10-27 - Loading States for Async Mutations
**Learning:** When adding visual loading states (`<Loader2 className="mr-2 h-4 w-4 animate-spin" />`) or disabling buttons for row-level actions in a mapped list sharing a `useMutation`, you must qualify the loading/disabled state using `mutation.variables === row.id` (or similar, depending on the mutation payload structure) to avoid disabling all items globally when any one row is mutating. Also, `import { Loader2 } from "lucide-react";` needs to be carefully merged with existing lucide-react imports if present to pass the TypeScript linter.
**Action:** Always check the payload structure of the `mutate` call and verify `mutation.isPending && mutation.variables === id` before showing row-level loading states.

## 2026-08-31 - Context-aware ARIA Labels in Tables
**Learning:** Interactive elements within repeating table rows (like 'show/hide' details buttons or pagination 'Prev/Next' buttons) often use brief, non-descriptive text that becomes confusing when announced out of context by a screen reader.
**Action:** Always add contextual `aria-label`s (e.g., `aria-label="Show details"`) and state indicators (e.g., `aria-expanded`, `aria-pressed`) to generic text buttons in mapped lists and tables.
