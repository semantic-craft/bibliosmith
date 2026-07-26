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
    // The two long-standing hooks rules are listed by hand rather than pulling
    // in eslint-plugin-react-hooks' recommended preset. As of v7 that preset
    // also turns on the React Compiler rule family (set-state-in-effect,
    // purity, immutability, refs, ...), and set-state-in-effect alone fires 14
    // times across App.tsx and BookDrawer.tsx. Clearing those means
    // restructuring effects in the app's core components, which is app
    // behaviour work rather than CI wiring; tracked separately so this file
    // does not quietly carry a disabled rule instead.
    plugins: { "react-hooks": reactHooks },
    rules: {
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "error",
    },
    languageOptions: {
      globals: globals.browser,
    },
  },
  {
    // The build and test scripts under scripts/ are plain node modules, not
    // browser code, and none of the React rules above apply to them.
    files: ["scripts/**/*.mjs", "*.config.{js,ts}"],
    languageOptions: {
      globals: globals.node,
    },
  },
);
