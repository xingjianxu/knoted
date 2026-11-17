import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useBlockNoteEditor } from "@blocknote/react";
import { BlockNoteEditor } from "@blocknote/core";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import type { components } from "@knoted/api-client";

import { NoteEditor } from "../../src/features/notes/editor/note-editor";
import { fromEditorBlocks, toEditorBlocks } from "../../src/features/notes/editor/document-adapter";
import { noteEditorSchema } from "../../src/features/notes/editor/schema";
import { UiProvider } from "../../src/lib/ui-provider";
import { pasteHtml, pastePlainText } from "../fixtures/editor-clipboard";

type NoteDocument = components["schemas"]["NoteDocument"];
const document: NoteDocument = { blocks: [
  { id: "h", type: "heading", level: 3, content: [{ type: "text", text: "中文标题", marks: ["bold", "italic", "underline", "strike", "code"] }] },
  { id: "p", type: "paragraph", content: [{ type: "text", text: "正文 " }, { type: "link", text: "链接", href: "https://example.com/%E4%B8%AD%E6%96%87", marks: ["bold", "code"] }] },
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
  it("allows paragraph Enter before BlockNote assigns the new block ID", () => {
    let editor!: ReturnType<typeof useBlockNoteEditor>;
    function Probe() { editor = useBlockNoteEditor(); return null; }
    const onChange = vi.fn();
    const view = render(<UiProvider><NoteEditor document={{ blocks: [{ id: "p", type: "paragraph", content: [{ type: "text", text: "原文" }] }] }} onChange={onChange}><Probe /></NoteEditor></UiProvider>);
    const editable = view.container.querySelector<HTMLElement>('[contenteditable="true"]')!;
    act(() => editor.setTextCursorPosition("p", "end"));
    fireEvent.keyDown(editable, { key: "Enter" });
    act(() => editor.insertInlineContent("新段正文"));
    const saved = onChange.mock.lastCall?.[0] as NoteDocument;
    expect(saved.blocks).toHaveLength(2);
    expect(saved.blocks[0]).toEqual({ id: "p", type: "paragraph", content: [{ type: "text", text: "原文" }] });
    expect(saved.blocks[1]).toMatchObject({ type: "paragraph", content: [{ type: "text", text: "新段正文" }] });
    expect(saved.blocks[1]?.id).toBeTruthy();
    expect(saved.blocks[1]?.id).not.toBe("p");
    expect(screen.queryByText(/未应用此次编辑/)).not.toBeInTheDocument();
  });

  it("validates new ID-less blocks in multi-paragraph paste without rejecting valid content", async () => {
    let editor!: ReturnType<typeof useBlockNoteEditor>;
    function Probe() { editor = useBlockNoteEditor(); return null; }
    const onChange = vi.fn();
    const view = render(<UiProvider><NoteEditor document={{ blocks: [{ id: "p", type: "paragraph", content: [{ type: "text", text: "原文" }] }] }} onChange={onChange}><Probe /></NoteEditor></UiProvider>);
    const editable = view.container.querySelector<HTMLElement>('[contenteditable="true"]')!;
    act(() => editor.setTextCursorPosition("p", "end"));
    pasteHtml(editable, '<p>第一段</p><p>第二段<a href="https://example.com/%E4%B8%AD#ok">有效链接</a></p>', "第一段\n第二段有效链接");
    const saved = onChange.mock.lastCall?.[0] as NoteDocument;
    expect(saved.blocks).toHaveLength(2);
    expect(saved.blocks[0]?.content).toEqual([{ type: "text", text: "原文第一段" }]);
    expect(saved.blocks[1]?.content).toEqual([{ type: "text", text: "第二段" }, { type: "link", text: "有效链接", href: "https://example.com/%E4%B8%AD#ok" }]);
    expect(saved.blocks.every(block => Boolean(block.id))).toBe(true);
    expect(new Set(saved.blocks.map(block => block.id)).size).toBe(2);
    // A malformed link in a newly created (initially ID-less) second paragraph
    // must cancel the entire next paste, including its valid first paragraph.
    const before = JSON.stringify(saved);
    pasteHtml(editable, '<p>不应插入</p><p><a href="https://example.com/#part#two">非法新块</a></p>', "不应插入\n非法新块");
    expect(await screen.findByText(/未应用此次编辑/)).toBeInTheDocument();
    expect(editable).not.toHaveTextContent("不应插入");
    expect(editable).not.toHaveTextContent("非法新块");
    expect(JSON.stringify(onChange.mock.lastCall?.[0])).toBe(before);
    act(() => editor.insertInlineContent("后续仍保稿"));
    expect(JSON.stringify(onChange.mock.lastCall?.[0])).toContain("后续仍保稿");
  });

  it("rejects a repeated-fragment paste atomically, then retains subsequent typing and valid encoded links", async () => {
    let editor!: ReturnType<typeof useBlockNoteEditor>;
    function Probe() { editor = useBlockNoteEditor(); return null; }
    const onChange = vi.fn();
    const initial: NoteDocument = { blocks: [{ id: "p", type: "paragraph", content: [{ type: "text", text: "原文" }] }] };
    const view = render(<UiProvider><NoteEditor document={initial} onChange={onChange}><Probe /></NoteEditor></UiProvider>);
    const editable = view.container.querySelector<HTMLElement>('[contenteditable="true"]')!;
    act(() => editor.setTextCursorPosition("p", "end"));
    const selection = editor.prosemirrorState.selection;
    pasteHtml(editable, '<p>不应部分插入<a href="https://example.com/#part#two">中文链接</a></p>', "不应部分插入中文链接");
    expect(await screen.findByText(/未应用此次编辑/)).toHaveTextContent(/未应用.*原内容已保留/);
    expect(editable).toHaveTextContent("原文");
    expect(editable).not.toHaveTextContent("中文链接");
    expect(editable).not.toHaveTextContent("不应部分插入");
    expect(editor.prosemirrorState.selection.eq(selection)).toBe(true);
    expect(onChange).not.toHaveBeenCalled();
    act(() => editor.insertInlineContent("后续正文"));
    expect(onChange.mock.lastCall?.[0]).toEqual({ blocks: [{ id: "p", type: "paragraph", content: [{ type: "text", text: "原文后续正文" }] }] });
    expect(view.container.querySelector('[contenteditable="true"]')).toBe(editable);
    expect(screen.queryByText(/未应用此次编辑/)).not.toBeInTheDocument();
    pasteHtml(editable, '<p><a href="https://example.com/%E4%B8%AD#part%23two">有效链接</a></p>', "有效链接");
    await waitFor(() => expect(JSON.stringify(onChange.mock.lastCall?.[0])).toContain("https://example.com/%E4%B8%AD#part%23two"));
    expect(editable).toHaveTextContent("有效链接");
    act(() => editor.undo());
    expect(editable).not.toHaveTextContent("有效链接");
    act(() => editor.redo());
    expect(editable).toHaveTextContent("有效链接");
    expect(JSON.stringify(onChange.mock.lastCall?.[0])).toContain("https://example.com/%E4%B8%AD#part%23two");
  });

  it("prevents a public link command from committing an unsafe URI without losing existing text", async () => {
    let editor!: ReturnType<typeof useBlockNoteEditor>;
    function Probe() { editor = useBlockNoteEditor(); return null; }
    const onChange = vi.fn();
    const view = render(<UiProvider><NoteEditor document={{ blocks: [{ id: "p", type: "paragraph", content: [{ type: "text", text: "保留正文" }] }] }} onChange={onChange}><Probe /></NoteEditor></UiProvider>);
    act(() => editor.setTextCursorPosition("p", "end"));
    act(() => editor.insertInlineContent([{ type: "link", href: "javascript:alert(1)", content: "危险链接" }]));
    expect(await screen.findByText(/未应用此次编辑/)).toBeInTheDocument();
    expect(view.container.querySelector('[contenteditable="true"]')).toHaveTextContent("保留正文");
    expect(view.container.querySelector('[contenteditable="true"]')).not.toHaveTextContent("危险链接");
    expect(onChange).not.toHaveBeenCalled();
  });

  it("keeps rejection feedback on selection changes and clears it after a delete-only edit", async () => {
    let editor!: ReturnType<typeof useBlockNoteEditor>;
    function Probe() { editor = useBlockNoteEditor(); return null; }
    const onChange = vi.fn();
    render(<UiProvider><NoteEditor document={{ blocks: [{ id: "p", type: "paragraph", content: [{ type: "text", text: "保留正文" }] }, { id: "remove", type: "paragraph", content: [{ type: "text", text: "待删除" }] }] }} onChange={onChange}><Probe /></NoteEditor></UiProvider>);
    act(() => editor.setTextCursorPosition("p", "end"));
    act(() => editor.insertInlineContent([{ type: "link", href: "https://example.com/#part#two", content: "非法链接" }]));
    expect(await screen.findByText(/未应用此次编辑/)).toBeInTheDocument();
    act(() => editor.setTextCursorPosition("p", "start"));
    expect(screen.getByText(/未应用此次编辑/)).toBeInTheDocument();
    act(() => editor.removeBlocks(["remove"]));
    expect(screen.queryByText(/未应用此次编辑/)).not.toBeInTheDocument();
    expect(onChange.mock.lastCall?.[0]).toEqual({ blocks: [{ id: "p", type: "paragraph", content: [{ type: "text", text: "保留正文" }] }] });
  });

  it("rejects an auto-linked invalid plain URL and accepts an explicit plain link label instead", async () => {
    const onChange = vi.fn();
    const view = render(<UiProvider><NoteEditor document={{ blocks: [{ id: "p", type: "paragraph", content: [{ type: "text", text: "原文" }] }] }} onChange={onChange} /></UiProvider>);
    const editable = view.container.querySelector<HTMLElement>('[contenteditable="true"]')!;
    editable.focus();
    const url = "https://example.com/#part#two";
    pasteHtml(editable, `<p><a href="${url}">${url}</a></p>`, url);
    expect(await screen.findByText(/未应用此次编辑/)).toBeInTheDocument();
    pastePlainText(editable, url);
    expect(editable).not.toHaveTextContent(url);
    expect(screen.getByText(/未应用此次编辑/)).toHaveTextContent(/仅以纯文本粘贴链接名称文字/);
    expect(onChange).not.toHaveBeenCalled();
    pastePlainText(editable, "链接名称文字");
    expect(editable).toHaveTextContent("链接名称文字");
    expect(editable.querySelector("a")).toBeNull();
    expect(screen.queryByText(/未应用此次编辑/)).not.toBeInTheDocument();
    expect(JSON.stringify(onChange.mock.lastCall?.[0])).toContain("链接名称文字");
    expect(JSON.stringify(onChange.mock.lastCall?.[0])).not.toContain('"type":"link"');
  });
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

  it("keeps code Enter/Tab within the code block and Shift-Enter deliberately exits", () => {
    const editor = BlockNoteEditor.create({ schema: noteEditorSchema, initialContent: [{ id: "code", type: "codeBlock", content: "one" }] });
    const host = window.document.createElement("div");
    window.document.body.append(host);
    editor.mount(host);
    try {
      editor.setTextCursorPosition("code", "end");
      const key = (key: string, shiftKey = false) => editor.prosemirrorView.dom.dispatchEvent(new KeyboardEvent("keydown", { key, shiftKey, bubbles: true }));
      key("Enter");
      expect(fromEditorBlocks(editor.document).blocks[0]?.content).toEqual([{ type: "text", text: "one\n" }]);
      key("Tab");
      expect(fromEditorBlocks(editor.document).blocks[0]?.content).toEqual([{ type: "text", text: "one\n  " }]);
      expect(editor.getTextCursorPosition().block.type).toBe("codeBlock");
      key("Enter", true);
      expect(editor.getTextCursorPosition().block.type).toBe("paragraph");
      expect(fromEditorBlocks(editor.document).blocks[0]?.content).toEqual([{ type: "text", text: "one\n  " }]);
      editor.setTextCursorPosition("code", "end");
      key("Enter"); key("Enter"); key("Enter");
      expect(editor.getTextCursorPosition().block.type).toBe("paragraph");
      expect(fromEditorBlocks(editor.document).blocks[0]?.content).toEqual([{ type: "text", text: "one\n  " }]);
    } finally { editor.unmount(); host.remove(); }
  });

  it("retains latest code/checkbox/heading/image values across type transitions and grouped undo/redo", () => {
    const source: NoteDocument = { blocks: [
      { id: "code", type: "codeBlock", language: "rust", checked: true },
      { id: "check", type: "checkListItem", checked: true, language: "untouched" },
      { id: "heading", type: "heading", level: 2 },
      { id: "image", type: "image", asset_id: "550e8400-e29b-41d4-a716-446655440000", language: "caption metadata" },
    ] };
    const editor = BlockNoteEditor.create({ schema: noteEditorSchema, initialContent: toEditorBlocks(source) });
    const host = window.document.createElement("div");
    window.document.body.append(host);
    editor.mount(host);
    try {
      editor.transact(() => {
        editor.updateBlock("code", { props: { language: "" } });
        editor.updateBlock("code", { type: "paragraph" });
        editor.updateBlock("check", { props: { checked: false } });
        editor.updateBlock("check", { type: "paragraph" });
        editor.updateBlock("heading", { props: { level: 3 } });
        editor.updateBlock("heading", { type: "paragraph" });
        editor.updateBlock("image", { props: { assetId: "550e8400-e29b-41d4-a716-446655440001" } });
        editor.updateBlock("image", { type: "paragraph" });
      });
      const expected: NoteDocument = { blocks: [
        { id: "code", type: "paragraph", language: "", checked: true },
        { id: "check", type: "paragraph", checked: false, language: "untouched" },
        { id: "heading", type: "paragraph", level: 3 },
        { id: "image", type: "paragraph", asset_id: "550e8400-e29b-41d4-a716-446655440001", language: "caption metadata" },
      ] };
      expect(fromEditorBlocks(editor.document, source)).toEqual(expected);
      editor.undo();
      expect(fromEditorBlocks(editor.document, source)).toEqual(source);
      editor.redo();
      expect(fromEditorBlocks(editor.document, source)).toEqual(expected);
    } finally { editor.unmount(); host.remove(); }
  });

  it.each(["https://example.com/中文", "https://例子.com", "https://example.com/%zz", "https://example.com/%2", "http:example.com", "https://example.com/a#b#c", "https://example.com/a[b]", "mailto:"])("rejects backend-invalid URI spelling: %s", (href) => {
    const source: NoteDocument = { blocks: [{ id: "p", type: "paragraph", content: [{ type: "link", text: "link", href }] }] };
    expect(() => toEditorBlocks(source)).toThrow();
    const editor = BlockNoteEditor.create({ schema: noteEditorSchema, initialContent: [{ id: "p", type: "paragraph", content: [{ type: "link", href, content: "link" }] }] });
    expect(() => fromEditorBlocks(editor.document)).toThrow();
    // The backend validates href whenever present, even on a text node.
    const textSource: NoteDocument = { blocks: [{ id: "p", type: "paragraph", content: [{ type: "text", text: "link", href }] }] };
    expect(() => toEditorBlocks(textSource)).toThrow();
    const plain = BlockNoteEditor.create({ schema: noteEditorSchema, initialContent: [{ id: "p", type: "paragraph", content: "link" }] });
    expect(() => fromEditorBlocks(plain.document, textSource)).toThrow();
  });

  it.each(["http://example.com/path?q=a#b", "HTTPS://example.com/%E4%B8%AD", "https://xn--fsqu00a.com/", "http://[::1]:8080/", "mailto:a@example.com?subject=hello%20world"])("preserves valid backend URI spelling: %s", (href) => {
    const source: NoteDocument = { blocks: [{ id: "p", type: "paragraph", content: [{ type: "link", text: "link", href }] }] };
    const editor = BlockNoteEditor.create({ schema: noteEditorSchema, initialContent: toEditorBlocks(source) });
    expect(fromEditorBlocks(editor.document)).toEqual(source);
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
