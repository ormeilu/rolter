import type { TestRunnerConfig } from "@storybook/test-runner";
import { getStoryContext } from "@storybook/test-runner";
import { checkA11y, injectAxe } from "axe-playwright";

// The viewport addon sizes the preview iframe inside the Storybook UI. The test
// runner drives the iframe directly, where nothing does, so a "fits at 375px"
// story would otherwise be measured at the browser default and assert nothing.
//
// `parameters.viewportSize` — set by `src/lib/story-viewport.ts` — is the width
// the story means; every other story is pinned to a desktop width so the one
// before it cannot leave the page narrow.
const DESKTOP = { width: 1280, height: 800 };

// every story is an accessibility test (#1181): after the play function has
// put the screen into its state, axe runs over the rendered root and fails
// the story on any serious or critical violation. moderate/minor findings
// stay informational so the gate catches regressions rather than colour
// contrast on a decorative chip. a story that needs an exception opts out
// with `parameters.a11y.disable = true` and says why beside it
const config: TestRunnerConfig = {
  async preVisit(page, context) {
    const story = await getStoryContext(page, context);
    const size = story.parameters?.viewportSize as
      | { width: number; height: number }
      | undefined;
    await page.setViewportSize(size ?? DESKTOP);
    await injectAxe(page);
  },
  async postVisit(page, context) {
    const story = await getStoryContext(page, context);
    if ((story.parameters?.a11y as { disable?: boolean } | undefined)?.disable) return;
    // the whole document rather than #storybook-root: dialogs, sheets and
    // toasts portal to <body>, and they are exactly what needs checking
    await checkA11y(page, undefined, {
      detailedReport: true,
      detailedReportOptions: { html: true },
      axeOptions: {
        runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "best-practice"] },
        // the iframe document is storybook's, not ours: it has no title or
        // lang, and the dashboard's index.html sets both
        rules: { "document-title": { enabled: false }, "html-has-lang": { enabled: false } },
      },
      includedImpacts: ["serious", "critical"],
    });
  },
};

export default config;
