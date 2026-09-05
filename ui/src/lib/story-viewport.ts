import { expect } from "storybook/test";

// Viewport fixtures for the responsive stories (#959, #1203).
//
// Two things have to agree for a "does it fit at 375px" story to mean
// anything. In the Storybook UI the viewport toolbar sizes the preview iframe;
// under the test runner nothing sizes it, so `.storybook/test-runner.ts` reads
// `parameters.viewportSize` and calls `page.setViewportSize` before the story
// renders. Both are set from the same constant here, so a story cannot claim a
// width in one place and be measured at another.
//
// Not a `.stories.tsx` file: it is a fixture, like `pages/story-harness.tsx`.

/** iPhone 12 mini / SE — the width #959 was reported at */
export const MOBILE = { width: 375, height: 812 } as const;
/** iPad portrait — the `md`…`lg` band where the rail is an icon strip */
export const TABLET = { width: 768, height: 1024 } as const;

const OPTIONS = {
  rolterMobile: { name: "Mobile 375", styles: { width: "375px", height: "812px" } },
  rolterTablet: { name: "Tablet 768", styles: { width: "768px", height: "1024px" } },
};

/** Story fields that pin a story to one of the two widths above. */
export const atMobile = {
  parameters: { viewportSize: MOBILE, viewport: { options: OPTIONS } },
  globals: { viewport: { value: "rolterMobile", isRotated: false } },
};

export const atTablet = {
  parameters: { viewportSize: TABLET, viewport: { options: OPTIONS } },
  globals: { viewport: { value: "rolterTablet", isRotated: false } },
};

/**
 * Nothing on the page is wider than the page.
 *
 * The assertion the audit in #959 could not make by eye: the shell clipped its
 * overflow, so `scrollWidth` read 375 while stat values were cut mid-digit.
 * Measuring the document rather than a screenshot is the whole point.
 */
export async function expectNoHorizontalOverflow(): Promise<void> {
  await expect(document.documentElement.scrollWidth).toBeLessThanOrEqual(window.innerWidth);
  await expect(document.body.scrollWidth).toBeLessThanOrEqual(window.innerWidth);
}
