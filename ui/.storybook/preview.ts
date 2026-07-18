import type { Preview } from "storybook-solidjs-vite";
// Load the generated design tokens + fonts so stories render with the real
// design system (Req 14.1). These are the same assets the app boots with.
import "../src/styles/tokens.generated.css";
import "../src/styles/fonts.css";

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
