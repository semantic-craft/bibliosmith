import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Kept separate from vite.config.ts so the dev/build config stays free of test
// concerns. vitest 4 accepts vite 8 as a peer, which is why it is the runner
// here rather than a standalone framework with its own transform pipeline.
export default defineConfig({
  plugins: [react()],
  test: {
    // jsdom for every file, including the pure-function suites: those import
    // from modules that also pull in DOM-touching siblings, and a second
    // environment would buy nothing back for a suite this size.
    environment: "jsdom",
    // No globals: each file imports describe/it/expect from "vitest", so
    // `tsc --noEmit` typechecks the tests with no extra ambient types.
    globals: false,
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
