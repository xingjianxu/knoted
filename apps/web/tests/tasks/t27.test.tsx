import type { BlockEditorBlock } from "@bixlab/mtone-blockeditor";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import type { components } from "@knoted/api-client";

import { NoteEditor } from "../../src/features/notes/editor/note-editor";
import {
  assetIdToUrl,
  fromEditorBlocks,
  toEditorBlocks,
  urlToAssetId,
} from "../../src/features/notes/editor/document-adapter";
import { UiProvider } from "../../src/lib/ui-provider";
import { pasteHtml, pastePlainText } from "../fixtures/editor-clipboard";

type NoteDocument = components["schemas"]["NoteDocument"];
const sampleAssetId = "550e8400-e29b-41d4-a716-446655440000";
const fullDocument: NoteDocument = {
  blocks: [
    {
      id: "h",
      type: "heading",
      level: 3,
      content: [
        { type: "text", text: "中文标题", marks: ["bold", "italic", "underline", "strike", "code"] },
      ],
    },
    {
      id: "p",
      type: "paragraph",
      content: [
        { type: "text", text: "正文 " },
        {
          type: "link",
          text: "链接",
          href: "https://example.com/%E4%B8%AD%E6%96%87",
          marks: ["bold", "code"],
        },
      ],
    },
    {
      id: "b",
      type: "bulletListItem",
      content: [{ type: "text", text: "一级" }],
      children: [
        {
          id: "n",
          type: "numberedListItem",
          content: [{ type: "text", text: "二级" }],
          children: [
            {
              id: "c",
              type: "checkListItem",
              checked: true,
              content: [{ type: "text", text: "待办" }],
            },
          ],
        },
      ],
    },
    {
      id: "i",
      type: "image",
      asset_id: sampleAssetId,
      content: [{ type: "text", text: "图注", marks: ["italic"] }],
    },
    {
      id: "code",
      type: "codeBlock",
      language: "rust",
      content: [{ type: "text", text: "  中文();\n\tlet x = 1;  \n", marks: ["bold"] }],
    },
  ],
};

function makeEditorBlocks(source: NoteDocument): BlockEditorBlock[] {
  const input = toEditorBlocks(source);
  return input.map((block) => materialize(block));
}

function materialize(block: ReturnType<typeof toEditorBlocks>[number]): BlockEditorBlock {
  const children = (block.children ?? []).map(materialize);
  switch (block.type) {
    case "heading":
      return {
        id: block.id!,
        type: "heading",
        props: {
          backgroundColor: "default",
          textColor: "default",
          textAlignment: "left",
          level: block.props?.level ?? 1,
          isToggleable: false,
        },
        content: (Array.isArray(block.content) ? block.content : []) as BlockEditorBlock["content"],
        children,
      } as BlockEditorBlock;
    case "checkListItem":
      return {
        id: block.id!,
        type: "checkListItem",
        props: {
          backgroundColor: "default",
          textColor: "default",
          textAlignment: "left",
          checked: Boolean(block.props?.checked),
        },
        content: (Array.isArray(block.content) ? block.content : []) as BlockEditorBlock["content"],
        children,
      } as BlockEditorBlock;
    case "codeBlock":
      return {
        id: block.id!,
        type: "codeBlock",
        props: { language: block.props?.language ?? "text" },
        content: typeof block.content === "string" && block.content
          ? [{ type: "text", text: block.content, styles: {} }]
          : [],
        children,
      } as BlockEditorBlock;
    case "image":
      return {
        id: block.id!,
        type: "image",
        props: {
          url: block.props?.url ?? "",
          alt: block.props?.alt ?? "",
          caption: block.props?.caption ?? "",
        },
        content: undefined,
        children,
      } as BlockEditorBlock;
    default:
      return {
        id: block.id!,
        type: block.type!,
        props: {
          backgroundColor: "default",
          textColor: "default",
          textAlignment: "left",
          ...(block.type === "numberedListItem" ? { start: undefined } : {}),
        },
        content: (Array.isArray(block.content) ? block.content : []) as BlockEditorBlock["content"],
        children,
      } as BlockEditorBlock;
  }
}

beforeAll(() => {
  Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
  Object.defineProperty(Range.prototype, "getBoundingClientRect", {
    configurable: true,
    value: () => new DOMRect(),
  });
});
afterAll(() => {
  Reflect.deleteProperty(Range.prototype, "getClientRects");
  Reflect.deleteProperty(Range.prototype, "getBoundingClientRect");
});

describe("T27 mtone BlockEditor integration & NoteDocument v1 adapter", () => {
  it("t27_2: preserves all seven blocks, five marks, marked links, nesting, code whitespace and asset IDs through the real editor", async () => {
    const roundTrip = fromEditorBlocks(makeEditorBlocks(fullDocument), fullDocument);
    expect(roundTrip).toEqual(fullDocument);
    expect(JSON.stringify(roundTrip)).not.toContain("/assets/");
    expect(JSON.stringify(roundTrip)).not.toContain("/api/");

    // Also round-trip through a real editable BlockEditor instance: editing a
    // trailing block must preserve every untouched block verbatim via onChange.
    const seeded: NoteDocument = {
      blocks: [
        { id: "tail", type: "paragraph", content: [{ type: "text", text: "起点" }] },
        ...fullDocument.blocks,
      ],
    };
    const onChange = vi.fn();
    const view = render(
      <UiProvider>
        <NoteEditor documentKey="roundtrip-real" document={seeded} onChange={onChange} />
      </UiProvider>,
    );
    const editable = await waitFor(() => {
      const el = view.container.querySelector<HTMLElement>('[contenteditable="true"]');
      expect(el).not.toBeNull();
      return el!;
    });
    editable.focus();
    pastePlainText(editable, "补充");
    await waitFor(() => expect(onChange).toHaveBeenCalled());
    const emitted = onChange.mock.lastCall![0] as NoteDocument;
    expect(emitted.blocks.slice(1)).toEqual(fullDocument.blocks);
    expect(JSON.stringify(emitted)).not.toContain("/assets/");
    expect(JSON.stringify(emitted)).not.toContain("/api/");
  });

  it("maps asset_id to same-origin HTTP(S) asset URL and rejects foreign origins or query strings", () => {
    const url = assetIdToUrl(sampleAssetId);
    expect(url).toBe(`${window.location.origin}/api/v1/assets/${sampleAssetId}/content`);
    expect(urlToAssetId(url)).toBe(sampleAssetId);
    expect(urlToAssetId(`/api/v1/assets/${sampleAssetId}/content`)).toBe(sampleAssetId);
    expect(urlToAssetId(`https://evil.example/api/v1/assets/${sampleAssetId}/content`)).toBeNull();
    expect(urlToAssetId(`${url}?download=1`)).toBeNull();
    expect(urlToAssetId(`${url}#frag`)).toBeNull();
    expect(urlToAssetId("https://example.com/photo.png")).toBeNull();
  });

  it("retains unchanged inline runs and serializes explicitly cleared code language", () => {
    const source: NoteDocument = {
      blocks: [
        {
          id: "p",
          type: "paragraph",
          content: [{ type: "text", text: "一", marks: [] }, { type: "text", text: "二" }],
          children: [],
        },
        { id: "code", type: "codeBlock", language: "rust", content: [{ type: "text", text: "fn main() {}" }] },
      ],
    };
    const blocks = makeEditorBlocks(source);
    expect(fromEditorBlocks(blocks, source)).toEqual(source);

    const clearedCode = blocks.map((block) =>
      block.id === "code" && block.type === "codeBlock"
        ? { ...block, props: { ...block.props, language: "text" } }
        : block,
    );
    expect(fromEditorBlocks(clearedCode, source).blocks[1]?.language).toBe("");
  });

  it.each([
    "https://example.com/中文",
    "https://例子.com",
    "https://example.com/%zz",
    "https://example.com/%2",
    "http:example.com",
    "https://example.com/a#b#c",
    "https://example.com/a[b]",
    "mailto:",
    "tel:+8613800138000",
    "javascript:alert(1)",
  ])("rejects backend-invalid URI spelling: %s", (href) => {
    const source: NoteDocument = {
      blocks: [{ id: "p", type: "paragraph", content: [{ type: "link", text: "link", href }] }],
    };
    expect(() => toEditorBlocks(source)).toThrow();
    const editorBlock = {
      id: "p",
      type: "paragraph",
      props: { backgroundColor: "default", textColor: "default", textAlignment: "left" },
      content: [{ type: "link", href, content: [{ type: "text", text: "link", styles: {} }] }],
      children: [],
    } as unknown as BlockEditorBlock;
    expect(() => fromEditorBlocks([editorBlock])).toThrow();
  });

  it.each([
    "http://example.com/path?q=a#b",
    "HTTPS://example.com/%E4%B8%AD",
    "https://xn--fsqu00a.com/",
    "http://[::1]:8080/",
    "mailto:a@example.com?subject=hello%20world",
  ])("preserves valid backend URI spelling: %s", (href) => {
    const source: NoteDocument = {
      blocks: [{ id: "p", type: "paragraph", content: [{ type: "link", text: "link", href }] }],
    };
    expect(fromEditorBlocks(makeEditorBlocks(source), source)).toEqual(source);
  });

  it("round trips table metadata, cell formats and empty cells", () => {
    const source: NoteDocument = {
      blocks: [{
        id: "t",
        type: "table",
        table: {
          header_rows: 1,
          header_columns: 1,
          column_widths: [120, null],
          rows: [
            { cells: [[{ type: "text", text: "标题", marks: ["bold", "code"] }], []] },
            { cells: [[{ type: "link", text: "详情", href: "https://example.com/" }], []] },
          ],
        },
      }],
    };
    const content = {
      type: "tableContent" as const,
      headerRows: 1,
      headerCols: 1,
      columnWidths: [120, undefined],
      rows: [
        { cells: [[{ type: "text" as const, text: "标题", styles: { code: true } }], []] },
        { cells: [[{ type: "link" as const, href: "https://example.com/", content: [
          { type: "text" as const, text: "详情", styles: {} },
        ] }], []] },
      ],
    };
    expect(toEditorBlocks(source)).toEqual([{ id: "t", type: "table", content, children: [] }]);
    const block: BlockEditorBlock = {
      id: "t", type: "table", props: { textColor: "default" }, content, children: [],
    };
    expect(fromEditorBlocks([block], source)).toEqual(source);
  });

  it("rejects a merged table before saving", () => {
    const merged: BlockEditorBlock = {
      id: "t",
      type: "table",
      props: { textColor: "default" },
      content: {
        type: "tableContent",
        columnWidths: [undefined, undefined],
        rows: [{ cells: [{
          type: "tableCell",
          props: { colspan: 2, rowspan: 1, backgroundColor: "default", textColor: "default", textAlignment: "left" },
          content: [{ type: "text", text: "合并", styles: {} }],
        }] }],
      },
      children: [],
    };
    expect(() => fromEditorBlocks([merged])).toThrow(/合并单元格/);
  });

  it("rejects unsupported BlockEditor block types, heading level > 3, external images and limit violations", () => {
    for (const type of ["quote", "divider", "toggleListItem", "callout", "video", "audio", "file"] as const) {
      const bad = { id: "u", type, props: {}, content: [], children: [] } as unknown as BlockEditorBlock;
      expect(() => fromEditorBlocks([bad])).toThrow(/暂不支持保存/);
    }
    const h4 = {
      id: "h4",
      type: "heading",
      props: { backgroundColor: "default", textColor: "default", textAlignment: "left", level: 4 },
      content: [{ type: "text", text: "四级标题", styles: {} }],
      children: [],
    } as unknown as BlockEditorBlock;
    expect(() => fromEditorBlocks([h4])).toThrow(/4–6 级标题/);

    const externalImg = {
      id: "img",
      type: "image",
      props: { url: "https://example.com/pic.png", alt: "", caption: "" },
      content: undefined,
      children: [],
    } as unknown as BlockEditorBlock;
    expect(() => fromEditorBlocks([externalImg])).toThrow(/上传图片/);

    let deep: BlockEditorBlock = {
      id: "leaf",
      type: "paragraph",
      props: { backgroundColor: "default", textColor: "default", textAlignment: "left" },
      content: [],
      children: [],
    } as unknown as BlockEditorBlock;
    for (let i = 8; i >= 1; i--) {
      deep = {
        id: `d-${i}`,
        type: "paragraph",
        props: { backgroundColor: "default", textColor: "default", textAlignment: "left" },
        content: [],
        children: [deep],
      } as unknown as BlockEditorBlock;
    }
    expect(() => fromEditorBlocks([deep])).toThrow(/超出保存限制/);
  });

  it("validates pasted HTML links through the real BlockEditor and keeps saved draft clean", async () => {
    const onChange = vi.fn();
    const initial: NoteDocument = {
      blocks: [{ id: "p", type: "paragraph", content: [{ type: "text", text: "原文" }] }],
    };
    const view = render(
      <UiProvider>
        <NoteEditor documentKey="paste-test" document={initial} onChange={onChange} />
      </UiProvider>,
    );
    const editable = await waitFor(() => {
      const el = view.container.querySelector<HTMLElement>('[contenteditable="true"]');
      expect(el).not.toBeNull();
      return el!;
    });
    editable.focus();

    pasteHtml(
      editable,
      '<p>第一段</p><p>第二段<a href="https://example.com/%E4%B8%AD#ok">有效链接</a></p>',
      "第一段\n第二段有效链接",
    );
    await waitFor(() => expect(onChange).toHaveBeenCalled());
    const saved = onChange.mock.lastCall?.[0] as NoteDocument;
    expect(JSON.stringify(saved)).toContain("https://example.com/%E4%B8%AD#ok");
    expect(screen.queryByText(/未应用此次编辑/)).not.toBeInTheDocument();

    const callCountBeforeInvalid = onChange.mock.calls.length;
    pasteHtml(
      editable,
      '<p><a href="https://example.com/#part#two">非法链接</a></p>',
      "非法链接",
    );
    expect(await screen.findByText(/未应用此次编辑/)).toHaveTextContent(/未应用.*原内容已保留.*纯文本粘贴链接名称/);
    expect(onChange).toHaveBeenCalledTimes(callCountBeforeInvalid);

    // Undo restores the valid editor state; subsequent plain paste saves cleanly.
    act(() => {
      fireEvent.keyDown(editable, { key: "z", ctrlKey: true });
    });
    pastePlainText(editable, "后续正文");
    await waitFor(() => expect(JSON.stringify(onChange.mock.lastCall?.[0])).toContain("后续正文"));
    expect(JSON.stringify(onChange.mock.lastCall?.[0])).not.toContain("#part#two");
    expect(screen.queryByText(/未应用此次编辑/)).not.toBeInTheDocument();
  });

  it("clears stale inputError when switching documentKey without remounting NoteEditor", async () => {
    const onChange = vi.fn();
    const initial: NoteDocument = {
      blocks: [{ id: "p", type: "paragraph", content: [{ type: "text", text: "旧文档" }] }],
    };
    const view = render(
      <UiProvider>
        <NoteEditor documentKey="doc-a" document={initial} onChange={onChange} />
      </UiProvider>,
    );
    const editable = await waitFor(() => {
      const el = view.container.querySelector<HTMLElement>('[contenteditable="true"]');
      expect(el).not.toBeNull();
      return el!;
    });
    editable.focus();
    pasteHtml(editable, '<p><a href="https://example.com/#part#two">非法链接</a></p>', "非法链接");
    expect(await screen.findByText(/未应用此次编辑/)).toBeInTheDocument();

    view.rerender(
      <UiProvider>
        <NoteEditor
          documentKey="doc-b"
          document={{ blocks: [{ id: "q", type: "paragraph", content: [{ type: "text", text: "新文档" }] }] }}
          onChange={onChange}
        />
      </UiProvider>,
    );
    expect(screen.queryByText(/未应用此次编辑/)).not.toBeInTheDocument();
    await waitFor(() => {
      const next = view.container.querySelector<HTMLElement>('[contenteditable="true"]');
      expect(next).toHaveTextContent("新文档");
    });
  });

  it("t27_1: preserves the editable DOM and focus across composition and parent rerenders", async () => {
    const onChange = vi.fn();
    const initial: NoteDocument = {
      blocks: [{ id: "p", type: "paragraph", content: [{ type: "text", text: "输入" }] }],
    };
    const view = render(
      <UiProvider>
        <NoteEditor documentKey="ime-1" document={initial} onChange={onChange} />
      </UiProvider>,
    );
    const editable = await waitFor(() => {
      const el = view.container.querySelector<HTMLElement>('[contenteditable="true"]');
      expect(el).not.toBeNull();
      return el!;
    });
    editable.focus();
    fireEvent.compositionStart(editable, { data: "中" });
    view.rerender(
      <UiProvider>
        <NoteEditor documentKey="ime-1" document={structuredClone(initial)} onChange={(value) => onChange(value)} />
      </UiProvider>,
    );
    expect(view.container.querySelector('[contenteditable="true"]')).toBe(editable);
    expect(editable).toHaveFocus();
    fireEvent.compositionEnd(editable, { data: "中文" });

    // Changing documentKey loads the new document into a fresh editor instance.
    view.rerender(
      <UiProvider>
        <NoteEditor
          documentKey="ime-2"
          document={{ blocks: [{ id: "p2", type: "paragraph", content: [{ type: "text", text: "新文档" }] }] }}
          onChange={onChange}
        />
      </UiProvider>,
    );
    await waitFor(() => {
      const next = view.container.querySelector<HTMLElement>('[contenteditable="true"]');
      expect(next).not.toBeNull();
      expect(next).not.toBe(editable);
      expect(next).toHaveTextContent("新文档");
    });
  });

  it("t27_3: renders mtone BlockEditor wrapper and supports readOnly presentation without edit controls", async () => {
    const onChange = vi.fn();
    const view = render(
      <UiProvider>
        <NoteEditor documentKey="ro" document={fullDocument} readOnly onChange={onChange} />
      </UiProvider>,
    );
    await waitFor(() => expect(view.container.querySelector(".mt-block-editor")).not.toBeNull());
    expect(view.container.querySelector('[contenteditable="false"]')).not.toBeNull();
    expect(view.container.querySelector('[contenteditable="true"]')).toBeNull();
    expect(view.container.querySelector('[class*="mantine"], [data-radix-popper-content-wrapper]')).toBeNull();
    expect(screen.getByText("中文标题")).toBeInTheDocument();
    const figure = screen.getByRole("figure");
    fireEvent.focus(figure);
    expect(await screen.findByRole("tooltip")).toHaveTextContent("图注");
    expect(onChange).not.toHaveBeenCalled();
  });
});
