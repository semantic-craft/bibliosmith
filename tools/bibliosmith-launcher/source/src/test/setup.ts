import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";

// Testing Library only auto-cleans when vitest globals are on; they are off
// here, so unmount between tests explicitly. Without this, a component test
// leaves its tree in the document and the next query matches two elements.
afterEach(cleanup);
