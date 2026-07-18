import type { StorybookConfig } from "storybook-solidjs-vite";

/**
 * Storybook config — the component-docs / design-system workbench (design.md
 * §1.20). Storybook is the design-sanctioned alternative to Histoire (Histoire
 * has no published SolidJS renderer). It shares the repo's Vite/Solid pipeline
 * via the `storybook-solidjs-vite` framework, so the kit is documented in the
 * same build the app uses.
 *
 * The kit primitives (task 0.4) add `*.stories.tsx` next to each component.
 */
const config: StorybookConfig = {
  stories: ["../src/**/*.stories.@(ts|tsx)", "../src/**/*.mdx"],
  addons: [],
  framework: {
    name: "storybook-solidjs-vite",
    options: {},
  },
  // ui/public (icon sprite, fonts) is served by Vite automatically; kept
  // explicit so stories can reference /icons/lucide-sprite.svg like the app.
  staticDirs: ["../public"],
};

export default config;
