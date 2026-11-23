import "@testing-library/jest-dom/vitest";

import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// jsdom has no layout; actual scroll restoration and resize behavior are covered in Chromium.
window.scrollTo = () => {};
Element.prototype.scrollTo = () => {};
if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver = class ResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

// Vitest runs without globals, so Testing Library cannot auto-register cleanup.
afterEach(cleanup);
