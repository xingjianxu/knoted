import "@testing-library/jest-dom/vitest";

import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// jsdom has no layout; actual scroll restoration is covered in Chromium.
window.scrollTo = () => {};

// Vitest runs without globals, so Testing Library cannot auto-register cleanup.
afterEach(cleanup);
