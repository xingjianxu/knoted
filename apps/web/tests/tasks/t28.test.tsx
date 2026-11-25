import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";

import { NoteEditor, type NoteDocument } from "../../src/features/notes/editor";
import { UiProvider } from "../../src/lib/ui-provider";

beforeAll(() => {
  vi.spyOn(window, "scrollBy").mockImplementation(() => {});
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(() => new DOMRect(10, 10, 500, 30));
  Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
  Object.defineProperty(Range.prototype, "getBoundingClientRect", {
    configurable: true,
    value: () => new DOMRect(),
  });
});
afterAll(() => {
  vi.restoreAllMocks();
  Reflect.deleteProperty(Range.prototype, "getClientRects");
  Reflect.deleteProperty(Range.prototype, "getBoundingClientRect");
  Reflect.deleteProperty(document, "elementsFromPoint");
});

async function mountEditor(
  initial: NoteDocument = {
    blocks: [
      { id: "p", type: "paragraph", content: [{ type: "text", text: "first" }] },
      { id: "q", type: "paragraph", content: [{ type: "text", text: "second" }] },
    ],
  },
) {
  const onChange = vi.fn();
  const view = render(
    <UiProvider>
      <NoteEditor documentKey="t28" document={initial} onChange={onChange} />
    </UiProvider>,
  );
  const editable = await waitFor(() => {
    const el = view.container.querySelector<HTMLElement>('[contenteditable="true"]');
    expect(el).not.toBeNull();
    return el!;
  });
  return { ...view, editable, onChange };
}

function triggerHoverSideMenu(container: HTMLElement, blockId: string) {
  const blockEl = container.querySelector<HTMLElement>(`[data-node-type="blockContainer"][data-id="${blockId}"]`);
  expect(blockEl).not.toBeNull();
  const contentEl = blockEl!.querySelector<HTMLElement>(".bn-block-content") ?? blockEl!;
  const editorEl = container.querySelector<HTMLElement>(".bn-editor")!;
  Object.defineProperty(document, "elementsFromPoint", {
    configurable: true,
    value: () => [contentEl, blockEl!, editorEl],
  });
  fireEvent.mouseMove(editorEl, { clientX: 20, clientY: 20 });
}

async function openSlashMenu(container: HTMLElement, blockId = "p") {
  triggerHoverSideMenu(container, blockId);
  const addButton = await screen.findByRole("button", { name: "添加块", hidden: true });
  fireEvent.mouseDown(addButton);
  fireEvent.click(addButton);
  const listbox = await screen.findByRole("listbox", { name: "插入块", hidden: true });
  await waitFor(() => expect(listbox.querySelectorAll('[role="option"]').length).toBeGreaterThan(1));
  return listbox;
}

describe("T28 mtone BlockEditor menus and interactions", () => {
  it("opens built-in slash menu from side add button, closes on Escape, and preserves content", async () => {
    const { container, editable, onChange } = await mountEditor();
    const listbox = await openSlashMenu(container, "p");

    expect(listbox).toHaveTextContent("一级标题");
    expect(listbox).toHaveTextContent("代码块");

    onChange.mockClear();
    fireEvent.keyDown(editable, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("listbox", { name: "插入块", hidden: true })).not.toBeInTheDocument());
    expect(onChange).not.toHaveBeenCalled();
    expect(editable).toHaveTextContent("first");
  });

  it("ignores Enter during IME composition while slash menu is open", async () => {
    const { container, editable, onChange } = await mountEditor();
    await openSlashMenu(container, "p");

    onChange.mockClear();
    fireEvent.compositionStart(editable, { data: "中" });
    fireEvent.keyDown(editable, { key: "Enter", keyCode: 229, isComposing: true });
    fireEvent.compositionEnd(editable, { data: "中文" });
    expect(onChange).not.toHaveBeenCalled();
    expect(document.getElementById("bn-suggestion-menu")).toBeInTheDocument();
  });

  it("inserts a supported heading block via built-in slash menu and emits valid NoteDocument", async () => {
    const { container, onChange } = await mountEditor({
      blocks: [{ id: "p", type: "paragraph" }],
    });
    const listbox = await openSlashMenu(container, "p");

    const headingOption = Array.from(listbox.querySelectorAll<HTMLElement>('[role="option"]')).find(
      (el) => el.textContent?.includes("一级标题"),
    )!;
    expect(headingOption).toBeDefined();
    fireEvent.mouseDown(headingOption);
    fireEvent.click(headingOption);

    await waitFor(() => expect(onChange).toHaveBeenCalled());
    const saved = onChange.mock.lastCall![0] as NoteDocument;
    expect(saved.blocks[0]).toMatchObject({ type: "heading", level: 1 });
    expect(screen.queryByText(/未应用此次编辑/)).not.toBeInTheDocument();
  });

  it("warns without calling onChange when inserting a block type unsupported by NoteDocument v1", async () => {
    const { container, onChange } = await mountEditor({
      blocks: [{ id: "p", type: "paragraph", content: [{ type: "text", text: "正文" }] }],
    });
    const listbox = await openSlashMenu(container, "p");

    const dividerOption = Array.from(listbox.querySelectorAll<HTMLElement>('[role="option"]')).find(
      (el) => el.textContent?.includes("分割线"),
    )!;
    expect(dividerOption).toBeDefined();
    onChange.mockClear();
    fireEvent.mouseDown(dividerOption);
    fireEvent.click(dividerOption);

    expect(await screen.findByText(/未应用此次编辑.*分割线/)).toBeInTheDocument();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("supports side block action menu (move down and delete block) through BlockEditor", async () => {
    const { container, onChange } = await mountEditor({
      blocks: [
        { id: "p", type: "paragraph", content: [{ type: "text", text: "first" }] },
        { id: "q", type: "paragraph", content: [{ type: "text", text: "second" }] },
      ],
    });
    triggerHoverSideMenu(container, "p");

    fireEvent.click(await screen.findByRole("button", { name: "此块的操作", hidden: true }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "下移" }));
    await waitFor(() => expect(onChange).toHaveBeenCalled());
    const moved = onChange.mock.lastCall![0] as NoteDocument;
    expect(moved.blocks.map((block) => block.id)).toEqual(["q", "p"]);

    onChange.mockClear();
    await act(async () => { await new Promise(requestAnimationFrame); });
    triggerHoverSideMenu(container, "q");
    const actionsButton = await waitFor(() => {
      const btn = document.querySelector<HTMLElement>('[aria-label="此块的操作"]');
      expect(btn).not.toBeNull();
      return btn!;
    });
    fireEvent.click(actionsButton);
    fireEvent.click(await screen.findByRole("menuitem", { name: "删除块" }));
    await waitFor(() => expect(onChange).toHaveBeenCalled());
    const afterDelete = onChange.mock.lastCall![0] as NoteDocument;
    expect(afterDelete.blocks.map((block) => block.id)).toEqual(["p"]);
  });
});
