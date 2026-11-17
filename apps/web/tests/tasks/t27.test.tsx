import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { BlockNoteEditor } from "@blocknote/core";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import type { components } from "@knoted/api-client";

import { NoteEditor } from "../../src/features/notes/editor/note-editor";
import { fromEditorBlocks, toEditorBlocks } from "../../src/features/notes/editor/document-adapter";
import { noteEditorSchema } from "../../src/features/notes/editor/schema";
import { UiProvider } from "../../src/lib/ui-provider";

type NoteDocument = components["schemas"]["NoteDocument"];
const document: NoteDocument = { blocks: [
  { id: "h", type: "heading", level: 3, content: [{ type: "text", text: "中文标题", marks: ["bold", "italic", "underline", "strike", "code"] }] },
  { id: "p", type: "paragraph", content: [{ type: "text", text: "正文 " }, { type: "link", text: "链接", href: "https://example.com/中文", marks: ["bold", "code"] }] },
  { id: "b", type: "bulletListItem", content: [{ type: "text", text: "一级" }], children: [
    { id: "n", type: "numberedListItem", content: [{ type: "text", text: "二级" }], children: [
      { id: "c", type: "checkListItem", checked: true, content: [{ type: "text", text: "待办" }] },
    ] },
  ] },
  { id: "i", type: "image", asset_id: "550e8400-e29b-41d4-a716-446655440000", content: [{ type: "text", text: "图注", marks: ["italic"] }] },
  { id: "code", type: "codeBlock", language: "rust", content: [{ type: "text", text: "  中文();\n\tlet x = 1;  \n", marks: ["bold"] }] },
] };

// jsdom has no layout. Supply only the missing Range geometry used when
// ProseMirror scrolls a selection; real selection geometry is checked in Chromium.
beforeAll(() => {
  Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
  Object.defineProperty(Range.prototype, "getBoundingClientRect", { configurable: true, value: () => new DOMRect() });
  const viewport = window.document.createElement("meta");
  viewport.name = "viewport";
  viewport.content = "width=device-width, initial-scale=1, interactive-widget=resizes-content";
  viewport.dataset.t27 = "true";
  window.document.head.append(viewport);
});
afterAll(() => {
  Reflect.deleteProperty(Range.prototype, "getClientRects");
  Reflect.deleteProperty(Range.prototype, "getBoundingClientRect");
  window.document.querySelector('meta[data-t27]')?.remove();
});

describe("T27 BlockNote editor", () => {
  it("t27_2: preserves all seven blocks, five marks, marked links, nesting, code whitespace and asset IDs through the real editor", () => {
    const editor = BlockNoteEditor.create({ schema: noteEditorSchema, initialContent: toEditorBlocks(document) });
    expect(fromEditorBlocks(editor.document)).toEqual(document);
    expect(JSON.stringify(fromEditorBlocks(editor.document))).not.toContain("/assets/");
  });

  it("retains backend optional fields and original inline runs while editing another property", () => {
    const source: NoteDocument = { blocks: [{ id: "p", type: "paragraph", level: 2, checked: false,
      language: "text", asset_id: "550e8400-e29b-41d4-a716-446655440000",
      content: [{ type: "text", text: "一", marks: [] }, { type: "text", text: "二" }], children: [],
    }] };
    const editor = BlockNoteEditor.create({ schema: noteEditorSchema, initialContent: toEditorBlocks(source) });
    expect(fromEditorBlocks(editor.document, source)).toEqual(source);
    editor.updateBlock("p", { type: "heading", props: { level: 3 } });
    expect(fromEditorBlocks(editor.document, source)).toEqual({ blocks: [{ ...source.blocks[0], type: "heading", level: 3 }] });
  });

  it("serializes explicitly cleared code language instead of restoring the source value", () => {
    const source: NoteDocument = { blocks: [{ id: "code", type: "codeBlock", language: "rust" }] };
    const editor = BlockNoteEditor.create({ schema: noteEditorSchema, initialContent: toEditorBlocks(source) });
    editor.updateBlock("code", { props: { language: "" } });
    expect(fromEditorBlocks(editor.document, source).blocks[0]?.language).toBe("");
  });

  it("rejects unsafe links and unsupported blocks rather than dropping saved content", () => {
    expect(() => toEditorBlocks({ blocks: [{ id: "bad", type: "paragraph", content: [{ type: "link", text: "bad", href: "javascript:alert(1)" }] }] })).toThrow();
    expect(() => toEditorBlocks({ blocks: [{ id: "bad", type: "table" }] } as unknown as NoteDocument)).toThrow();
  });

  it("t27_1: preserves the editable DOM and focus across composition and parent rerenders", async () => {
    const onChange = vi.fn();
    const initial: NoteDocument = { blocks: [{ id: "p", type: "paragraph", content: [{ type: "text", text: "输入" }] }] };
    const view = render(<UiProvider><NoteEditor document={initial} onChange={onChange} /></UiProvider>);
    const editable = view.container.querySelector<HTMLElement>('[contenteditable="true"]')!;
    expect(editable).not.toBeNull();
    editable.focus();
    fireEvent.compositionStart(editable, { data: "中" });
    view.rerender(<UiProvider><NoteEditor document={structuredClone(initial)} onChange={(value) => onChange(value)} /></UiProvider>);
    expect(view.container.querySelector('[contenteditable="true"]')).toBe(editable);
    expect(editable).toHaveFocus();
    fireEvent.compositionEnd(editable, { data: "中文" });
    fireEvent.mouseDown(screen.getByRole("button", { name: "标题 2" }));
    fireEvent.click(screen.getByRole("button", { name: "标题 2" }));
    await waitFor(() => expect(onChange).toHaveBeenCalled());
    expect(onChange.mock.lastCall?.[0].blocks[0]).toMatchObject({ type: "heading", level: 2 });
    fireEvent.click(screen.getByRole("button", { name: "撤销" }));
    await waitFor(() => expect(onChange.mock.lastCall?.[0].blocks[0].type).toBe("paragraph"));
  });

  it("t27_3: renders no default BlockNote UI and only supports the persisted whitelist", () => {
    const view = render(<UiProvider><NoteEditor document={document} onChange={() => {}} /></UiProvider>);
    expect(screen.getAllByRole("toolbar")).toHaveLength(1);
    expect(screen.queryByRole("button", { name: /Add block|Drag to move|添加块/ })).not.toBeInTheDocument();
    expect(view.container.querySelector('[class*="mantine"], [data-radix-popper-content-wrapper]')).toBeNull();
    expect(Object.keys(noteEditorSchema.blockSchema).sort()).toEqual(["bulletListItem", "checkListItem", "codeBlock", "heading", "image", "numberedListItem", "paragraph"]);
    expect(Object.keys(noteEditorSchema.styleSchema).sort()).toEqual(["bold", "code", "italic", "strike", "underline"]);
  });
});
