import { MtoneProvider, ToastProvider, type Tokens } from "@bixlab/mtone";
import type { ReactNode } from "react";

// Public context tokens also reach MtoneScope instances in body-level portals.
const tokens: Tokens = {
  fontFamily: "var(--font-sans)",
  "--mt-fg": "var(--color-ink)",
  "--mt-text": "var(--color-ink)",
  "--mt-primary": "var(--color-accent)",
  "--mt-muted": "var(--color-muted)",
  "--mt-bg": "var(--color-surface)",
  "--mt-border": "var(--color-line)",
  "--mt-focus": "var(--color-accent)",
};

export function UiProvider({ children }: { children: ReactNode }) {
  return (
    <MtoneProvider locale="zh" density="compact" tokens={tokens}>
      <ToastProvider>{children}</ToastProvider>
    </MtoneProvider>
  );
}
