import { MtoneScope } from "@bixlab/mtone";
import { createPortal } from "react-dom";
import { createRoot } from "react-dom/client";

import "../../src/styles.css";
import { UiProvider } from "../../src/lib/ui-provider";
import { T17Form } from "./t17-form";

createRoot(document.getElementById("root")!).render(
  <UiProvider>
    <T17Form />
    {createPortal(
      <MtoneScope role="note" aria-label="body-level scope">
        Portal 字体与主题探针
      </MtoneScope>,
      document.body,
    )}
  </UiProvider>,
);
