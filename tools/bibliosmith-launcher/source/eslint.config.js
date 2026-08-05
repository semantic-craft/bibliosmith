import js from "@eslint/js";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";

// The typechecker already covers types, so this config targets what tsc does
// not see: unused bindings, unreachable or duplicated code, and the rules of
// hooks. Type-aware linting is deliberately off — it needs a second full
// program build per run and would roughly double this job's time for rules the
// typechecker mostly already enforces.
export default tseslint.config(
  {
    ignores: ["dist/**", "src-tauri/**", "node_modules/**"],
  },
  js.configs.recommended,
  tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    ...reactHooks.configs.flat["recommended-latest"],
    languageOptions: {
      globals: globals.browser,
    },
  },
  {
    // The build and test scripts under scripts/ are plain node modules, not
    // browser code, and none of the React rules above apply to them.
    files: ["scripts/**/*.{js,cjs,mjs}", "*.config.{js,ts}"],
    languageOptions: {
      globals: globals.node,
    },
  },
  {
    files: ["scripts/**/*.{js,cjs}"],
    rules: {
      "@typescript-eslint/no-require-imports": "off",
    },
  },
);
