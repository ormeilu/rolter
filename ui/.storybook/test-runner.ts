import type { TestRunnerConfig } from "@storybook/test-runner";
import { getStoryContext } from "@storybook/test-runner";

// The viewport addon sizes the preview iframe inside the Storybook UI. The test
// runner drives the iframe directly, where nothing does, so a "fits at 375px"
// story would otherwise be measured at the browser default and assert nothing.
//
// `parameters.viewportSize` — set by `src/lib/story-viewport.ts` — is the width
// the story means; every other story is pinned to a desktop width so the one
// before it cannot leave the page narrow.
const DESKTOP = { width: 1280, height: 800 };

const config: TestRunnerConfig = {
  async preVisit(page, context) {
    const story = await getStoryContext(page, context);
    const size = story.parameters?.viewportSize as
      | { width: number; height: number }
      | undefined;
    await page.setViewportSize(size ?? DESKTOP);
  },
};

export default config;
