import type { Preview } from "storybook-solidjs-vite";
// Load the generated design tokens + fonts so stories render with the real
// design system (Req 14.1). These are the same assets the app boots with.
import "../src/styles/tokens.generated.css";
import "../src/styles/fonts.css";
// …and the base layer, which is what actually paints the page surface and base text
// colour from those tokens.
//
// Without it, tokens resolved to their DARK values (they are defined on `:root`, with
// `[data-theme="light"]` as the override) while the page kept the browser's white
// background — so every story rendered dark-on-white and looked washed out. That is
// easy to mistake for a contrast bug in the component under review; it was the
// workbench missing a stylesheet the app always loads.
import "../src/styles/base.css";
import "../src/styles/global.css";

const preview: Preview = {
  parameters: {
    controls: {
      matchers: {
        color: /(background|color)$/i,
        date: /Date$/i,
      },
    },
    backgrounds: {
      default: "kria-dark",
      values: [
        { name: "kria-dark", value: "#0c1216" },
        { name: "kria-light", value: "#f8fbff" },
      ],
    },
  },
};

export default preview;
