import { MtoneProvider, ToastProvider } from "@bixlab/mtone";
import type { ReactNode } from "react";

export function UiProvider({ children }: { children: ReactNode }) {
  return (
    <MtoneProvider locale="zh" density="compact">
      <ToastProvider>{children}</ToastProvider>
    </MtoneProvider>
  );
}
