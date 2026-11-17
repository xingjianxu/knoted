import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { SideMenuExtension, SuggestionMenu } from "@blocknote/core/extensions";
import { useBlockNoteEditor } from "@blocknote/react";
import { useEffect } from "react";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";

import { NoteEditor, noteEditorSchema, type NoteDocument, type NoteEditorInstance } from "../../src/features/notes/editor";
import { BlockMenu, SlashMenu } from "../../src/features/notes/editor/menus";
import { UiProvider } from "../../src/lib/ui-provider";

beforeAll(() => {
  vi.spyOn(window, "scrollBy").mockImplementation(() => {});
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(() => new DOMRect(10, 10, 500, 30));
  Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
  Object.defineProperty(Range.prototype, "getBoundingClientRect", { configurable: true, value: () => new DOMRect() });
  const meta = document.createElement("meta");
  meta.name = "viewport"; meta.content = "width=device-width, initial-scale=1, interactive-widget=resizes-content";
  meta.dataset.t28 = "true"; document.head.append(meta);
});
afterAll(() => {
  vi.restoreAllMocks();
  Reflect.deleteProperty(Range.prototype, "getClientRects");
  Reflect.deleteProperty(Range.prototype, "getBoundingClientRect");
  document.querySelector('meta[data-t28]')?.remove();
});

async function setup(initial: NoteDocument = { blocks: [
  { id: "p", type: "paragraph", content: [{ type: "text", text: "first" }] },
  { id: "q", type: "paragraph", content: [{ type: "text", text: "second" }] },
] }) {
  let editor!: NoteEditorInstance;
  function Probe() {
    const current = useBlockNoteEditor(noteEditorSchema);
    useEffect(() => { editor = current; }, [current]);
    return null;
  }
  const onChange = vi.fn();
  const view = render(<UiProvider><NoteEditor document={initial} onChange={onChange}><SlashMenu /><BlockMenu /><Probe /></NoteEditor></UiProvider>);
  await waitFor(() => expect(editor).toBeDefined());
  const editable = view.container.querySelector<HTMLElement>('[contenteditable="true"]')!;
  async function slash(query = "") {
    await act(async () => { await new Promise(requestAnimationFrame); });
    act(() => {
      editor.setTextCursorPosition("p", "end");
      editor.getExtension(SuggestionMenu)!.openSuggestionMenu("/", { deleteTriggerCharacter: true });
      if (query) editor.insertInlineContent(query);
    });
    await screen.findByLabelText("插入块");
    await waitFor(() => expect(editable).toHaveAttribute("aria-expanded", "true"));
    onChange.mockClear();
  }
  async function block(id = "p") {
    await act(async () => { await new Promise(requestAnimationFrame); });
    // jsdom has no hit testing. Supply only the public side-menu geometry/state;
    // Chromium evidence separately exercises real pointer detection/anchoring.
    act(() => editor.getExtension(SideMenuExtension)!.store.setState({ show: true, block: editor.getBlock(id)!, referencePos: new DOMRect() }));
    const trigger = await screen.findByLabelText("块操作");
    fireEvent.click(trigger);
    await screen.findByRole("menu");
    onChange.mockClear();
    return trigger;
  }
  return { editor, onChange, editable, slash, block };
}

describe("T28 actual BlockNote + mtone menus", () => {
  it("t28_1: Esc closes slash suggestions without changing the selection or document", async () => {
    const { editor, onChange, editable, slash } = await setup();
    await slash();
    const before = editor.prosemirrorState.selection.toJSON();
    fireEvent.keyDown(editable, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("listbox")).not.toBeInTheDocument());
    expect(editor.prosemirrorState.selection.toJSON()).toEqual(before);
    expect(onChange).not.toHaveBeenCalled();
    expect(editable).toHaveFocus();
  });

  it("t28_2/t28_3: filtered keyboard insertion clears slash/query in one change and one undo", async () => {
    const { editor, onChange, editable, slash } = await setup();
    await slash("heading");
    await waitFor(() => expect(screen.getAllByRole("option", { hidden: true })).toHaveLength(3));
    fireEvent.keyDown(editable, { key: "ArrowDown" });
    fireEvent.keyDown(editable, { key: "Enter" });
    await waitFor(() => expect(onChange).toHaveBeenCalledTimes(1));
    const saved = onChange.mock.lastCall![0] as NoteDocument;
    // Nonempty current content stays intact; selected block is inserted after it.
    expect(saved.blocks[0]).toEqual({ id: "p", type: "paragraph", content: [{ type: "text", text: "first" }] });
    expect(saved.blocks[1]).toMatchObject({ type: "heading", level: 2 });
    expect(editor.getTextCursorPosition().block.type).toBe("heading");
    expect(editable).toHaveFocus();
    act(() => { editor.undo(); });
    expect(editor.document.some((b) => b.type === "heading")).toBe(false);
  });

  it("empty query offers valid blocks and converts an empty block without a temporary image", async () => {
    const { editor, onChange, editable, slash } = await setup({ blocks: [{ id: "p", type: "paragraph" }] });
    await slash();
    expect(screen.queryByRole("option", { name: /图片/ })).not.toBeInTheDocument();
    fireEvent.keyDown(editable, { key: "ArrowDown" });
    fireEvent.keyDown(editable, { key: "Enter" });
    await waitFor(() => expect(onChange).toHaveBeenCalledTimes(1));
    expect(onChange.mock.lastCall![0].blocks[0]).toMatchObject({ id: "p", type: "heading", level: 1 });
    expect(editor.document).toHaveLength(1);
  });

  it("unknown query closes suggestions without dropping typed text", async () => {
    const { editor, editable, slash } = await setup();
    await slash();
    act(() => editor.insertInlineContent("no-such-command"));
    await waitFor(() => expect(screen.queryByRole("listbox")).not.toBeInTheDocument());
    expect(editable.textContent).toContain("first/no-such-command");
  });

  it("block Esc restores focus and preserves an existing text selection", async () => {
    const { editor, onChange, editable, block } = await setup();
    act(() => editor.setSelection("p", "q"));
    const before = editor.prosemirrorState.selection.toJSON();
    await block();
    fireEvent.keyDown(screen.getAllByRole("menuitem")[0]!, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("menu")).not.toBeInTheDocument());
    await waitFor(() => expect(editable).toHaveFocus());
    expect(editor.prosemirrorState.selection.toJSON()).toEqual(before);
    expect(onChange).not.toHaveBeenCalled();
  });

  it.each([
    ["转换为待办", "checkListItem"], ["转换为代码块", "codeBlock"], ["转换为标题 3", "heading"],
  ])("%s preserves text and causes one onChange", async (label, type) => {
    const { onChange, editable, block } = await setup();
    await block();
    fireEvent.click(screen.getByRole("menuitem", { name: label }));
    await waitFor(() => expect(onChange).toHaveBeenCalledTimes(1));
    expect(onChange.mock.lastCall![0].blocks[0]).toMatchObject({ id: "p", type, content: [{ type: "text", text: "first" }] });
    await waitFor(() => expect(editable).toHaveFocus());
  });

  it("disables a move past the document boundary instead of issuing an empty change", async () => {
    const { onChange, block } = await setup();
    await block();
    expect(screen.getByRole("menuitem", { name: "上移" })).toHaveAttribute("aria-disabled", "true");
    fireEvent.click(screen.getByRole("menuitem", { name: "上移" }));
    expect(onChange).not.toHaveBeenCalled();
  });

  it.each(["上移", "下移"])("rejects %s that would deepen the whole subtree from 8 to 9 levels", async (label) => {
    let tree: NoteDocument["blocks"][number] = { id: "leaf", type: "paragraph" };
    for (let i = 6; i >= 1; i--) tree = { id: `level-${i}`, type: "paragraph", children: [tree] };
    const blocks: NoteDocument["blocks"] = [
      { id: "previous", type: "paragraph", children: [{ id: "existing", type: "paragraph" }] },
      { id: "p", type: "paragraph", children: [tree] },
    ];
    if (label === "下移") blocks.reverse();
    const { editor, onChange, block } = await setup({ blocks });
    await block();
    expect(screen.getByRole("menuitem", { name: label })).toHaveAttribute("aria-disabled", "true");
    fireEvent.click(screen.getByRole("menuitem", { name: label }));
    expect(onChange).not.toHaveBeenCalled();
    expect(editor.document.map((b) => b.id)).toEqual(label === "上移" ? ["previous", "p"] : ["p", "previous"]);
  });

  it("permits the native move into children exactly at the 8-level boundary", async () => {
    let tree: NoteDocument["blocks"][number] = { id: "leaf", type: "paragraph" };
    for (let i = 5; i >= 1; i--) tree = { id: `level-${i}`, type: "paragraph", children: [tree] };
    const { editor, onChange, block } = await setup({ blocks: [
      { id: "previous", type: "paragraph", children: [{ id: "existing", type: "paragraph" }] },
      { id: "p", type: "paragraph", children: [tree] },
    ] });
    await block();
    fireEvent.click(screen.getByRole("menuitem", { name: "上移" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledTimes(1));
    expect(editor.document).toHaveLength(1);
    expect(editor.document[0]!.children.map((b) => b.id)).toEqual(["existing", "p"]);
  });

  it("at 5000 blocks disables insertion before deleting query but permits empty-block conversion", async () => {
    const blocks: NoteDocument["blocks"] = Array.from({ length: 5000 }, (_, i) => ({ id: i === 0 ? "p" : `b-${i}`, type: "paragraph" }));
    blocks[0]!.content = [{ type: "text", text: "first" }];
    const { editor, onChange, editable, slash } = await setup({ blocks });
    await slash("heading");
    expect(screen.getAllByRole("option", { hidden: true })[0]).toHaveAttribute("aria-disabled", "true");
    fireEvent.keyDown(editable, { key: "Enter" });
    expect(onChange).not.toHaveBeenCalled();
    expect(editor.getBlock("p")!.content).toEqual([{ type: "text", text: "first/heading", styles: {} }]);
    act(() => { editor.getExtension(SuggestionMenu)!.closeMenu(); editor.updateBlock("p", { content: [] }); });
    await slash("heading");
    fireEvent.keyDown(editable, { key: "Enter" });
    await waitFor(() => expect(onChange).toHaveBeenCalledTimes(1));
    expect(editor.document).toHaveLength(5000);
    expect(editor.getBlock("p")!.type).toBe("heading");
  }, 15000);

  it("does not accept slash commands during IME composition", async () => {
    const { onChange, editable, slash } = await setup();
    await slash();
    fireEvent.compositionStart(editable, { data: "中" });
    fireEvent.keyDown(editable, { key: "Enter", isComposing: true });
    fireEvent.compositionEnd(editable, { data: "中文" });
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByRole("listbox", { hidden: true })).toBeInTheDocument();
  });

  it("move/delete each emit once, preserve nested content and are undoable", async () => {
    const { editor, onChange, block } = await setup({ blocks: [
      { id: "p", type: "paragraph", children: [{ id: "child", type: "bulletListItem", content: [{ type: "text", text: "nested" }] }] },
      { id: "q", type: "paragraph" },
    ] });
    await block();
    fireEvent.click(screen.getByRole("menuitem", { name: "下移" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledTimes(1));
    expect(editor.document.map((b) => b.id)).toEqual(["q", "p"]);
    expect(editor.document[1]!.children[0]!.id).toBe("child");
    act(() => editor.undo());
    expect(editor.document.map((b) => b.id)).toEqual(["p", "q"]);
    await block();
    fireEvent.click(screen.getByRole("menuitem", { name: "删除块" }));
    await waitFor(() => expect(onChange).toHaveBeenCalledTimes(1));
    expect(editor.document.map((b) => b.id)).toEqual(["q"]);
    act(() => editor.undo());
    expect(editor.document[0]!.children[0]!.id).toBe("child");
  });
});
