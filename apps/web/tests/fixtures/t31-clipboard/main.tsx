import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createMemoryHistory } from "@tanstack/react-router";
import App from "../../../src/App";
import { createAppRouter } from "../../../src/lib/router";
import { UiProvider } from "../../../src/lib/ui-provider";
import "../../../src/styles.css";

// The browser harness serves HTTP fixtures; the page uses the actual client,
// notes route, autosave sessions and editor with no test hooks in production.
const router = createAppRouter({ history: createMemoryHistory({ initialEntries: ["/notes/00000000-0000-4000-8000-000000000001"] }) });
createRoot(document.getElementById("root")!).render(<StrictMode><UiProvider><App router={router} /></UiProvider></StrictMode>);
