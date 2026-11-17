import { MtoneScope } from "@bixlab/mtone";
import { StrictMode } from "react";
import { createPortal } from "react-dom";
import { createRoot } from "react-dom/client";

import "../../src/styles.css";
import { UiProvider } from "../../src/lib/ui-provider";
import { T17Form } from "./t17-form";
import { T18Overlays } from "./t18-overlays";
import { T19TreeList } from "./t19-tree-list";
import { T27Editor } from "./t27-editor";
import { T28Editor } from "./t28-editor";

const task = new URLSearchParams(window.location.search).get("task");

function renderTask() {
  if (task === "t28") {
    return <StrictMode><UiProvider><T28Editor /></UiProvider></StrictMode>;
  }
  if (task === "t27") {
    return <StrictMode><UiProvider><T27Editor /></UiProvider></StrictMode>;
  }
  if (task === "t18") {
    return (
      <StrictMode>
        <UiProvider>
          <T18Overlays />
        </UiProvider>
      </StrictMode>
    );
  }
  if (task === "t19") {
    return (
      <StrictMode>
        <UiProvider>
          <T19TreeList />
        </UiProvider>
      </StrictMode>
    );
  }
  return (
    <UiProvider>
      <T17Form />
      {createPortal(
        <MtoneScope role="note" aria-label="body-level scope">
          Portal 字体与主题探针
        </MtoneScope>,
        document.body,
      )}
    </UiProvider>
  );
}

createRoot(document.getElementById("root")!).render(renderTask());
