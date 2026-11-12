import js from "@eslint/js";
import globals from "globals";
import jsxA11y from "eslint-plugin-jsx-a11y";
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["dist"] },
  {
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: {
      ecmaVersion: 2022,
      globals: globals.browser,
    },
    plugins: {
      "jsx-a11y": jsxA11y,
      "react-hooks": reactHooks,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      // Accessibility in the markup, before a browser runs (UI-004); the axe
      // specs in ui-tests check the rendered pages.
      ...jsxA11y.flatConfigs.recommended.rules,
      // A block of code scrolls sideways, so it takes keyboard focus (axe's
      // scrollable-region-focusable).
      "jsx-a11y/no-noninteractive-tabindex": ["error", { tags: ["pre"], roles: ["tabpanel"], allowExpressionValues: true }],
    },
  },
);
