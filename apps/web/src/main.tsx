import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import App from "./App";
import { UiProvider } from "./lib/ui-provider";
import "./styles.css";

const container = document.getElementById("root");

if (!container) {
  throw new Error('Missing "#root" element');
}

createRoot(container).render(
  <StrictMode>
    <UiProvider>
      <App />
    </UiProvider>
  </StrictMode>,
);
