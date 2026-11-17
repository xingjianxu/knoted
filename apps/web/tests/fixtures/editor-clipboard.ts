import { fireEvent } from "@testing-library/react";

/** jsdom clipboard payload only; parsing and transactions stay in real BlockNote. */
export function pasteHtml(editable: HTMLElement, html: string, text: string) {
  paste(editable, { "text/html": html, "text/plain": text });
}

export function pastePlainText(editable: HTMLElement, text: string) {
  paste(editable, { "text/plain": text });
}

function paste(editable: HTMLElement, formats: Record<string, string>) {
  const missing = typeof ClipboardEvent === "undefined";
  if (missing) Object.defineProperty(globalThis, "ClipboardEvent", { configurable: true, value: class extends Event {
    clipboardData: DataTransfer | null;
    constructor(type: string, options: ClipboardEventInit = {}) {
      super(type, options); this.clipboardData = options.clipboardData ?? null;
    }
  } });
  try { fireEvent.paste(editable, { clipboardData: {
    getData: (type: string) => formats[type] ?? "",
    types: Object.keys(formats), files: [], items: [],
  } }); } finally { if (missing) Reflect.deleteProperty(globalThis, "ClipboardEvent"); }
}
