import type { components } from "@knoted/api-client";

import type { EditorBlock, EditorPartialBlock } from "./schema";

export type NoteDocument = components["schemas"]["NoteDocument"];
type NoteBlock = components["schemas"]["NoteBlock"];
type InlineNode = components["schemas"]["InlineNode"];
type Mark = components["schemas"]["InlineMark"];
const marks: Mark[] = ["bold", "italic", "underline", "strike", "code"];
const blockTypes: NoteBlock["type"][] = ["heading", "paragraph", "bulletListItem", "numberedListItem", "checkListItem", "image", "codeBlock"];

function safeHref(href: string): string {
  if (/\s/u.test(href) || [...href].some((char) => char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127)) throw new Error("链接地址无效");
  const url = new URL(href);
  if (!["http:", "https:", "mailto:"].includes(url.protocol)) throw new Error("链接协议不受支持");
  return href;
}

/** Explicit DTO boundary: BlockNote props/styles/link.content never reach HTTP. */
export function toEditorBlocks(document: NoteDocument): EditorPartialBlock[] {
  const ids = new Set<string>();
  let count = 0;
  function convert(block: NoteBlock, depth: number): EditorPartialBlock {
    if (!blockTypes.includes(block.type)) throw new Error("不支持的笔记块");
    if (!block.id || [...block.id].length > 64 || ids.has(block.id)) throw new Error("笔记块 ID 无效");
    ids.add(block.id);
    if (++count > 5000 || depth > 8) throw new Error("笔记块超出限制");
    if (block.type === "heading" && ![1, 2, 3].includes(block.level ?? 0)) throw new Error("标题级别无效");
    if (block.type === "image" && !block.asset_id) throw new Error("图片缺少资产 ID");
    const content = (block.content ?? []).map((node) => {
      if (node.type !== "text" && node.type !== "link") throw new Error("不支持的行内格式");
      const styles: Partial<Record<Mark, boolean>> = {};
      for (const mark of node.marks ?? []) {
        if (!marks.includes(mark)) throw new Error("不支持的文字格式");
        styles[mark] = true;
      }
      const text = { type: "text" as const, text: node.text, styles };
      return node.type === "link"
        ? { type: "link" as const, href: safeHref(node.href ?? ""), content: [text] }
        : text;
    });
    return {
      id: block.id,
      type: block.type,
      props: {
        ...(block.type === "heading" ? { level: block.level } : {}),
        ...(block.type === "checkListItem" ? { checked: block.checked ?? false } : {}),
        ...(block.type === "codeBlock" ? { language: block.language ?? "" } : {}),
        ...(block.type === "image" ? { assetId: block.asset_id } : {}),
      },
      content,
      children: (block.children ?? []).map((child) => convert(child, depth + 1)),
    } as EditorPartialBlock;
  }
  return document.blocks.map((block) => convert(block, 1));
}

// Keep the source DTO as sidecar state, never in the editable HTML. Backend
// permits optional fields on every block type; preserve them when another block
// property changes, and retain unchanged inline run boundaries/mark ordering.
export function fromEditorBlocks(blocks: EditorBlock[], source?: NoteDocument): NoteDocument {
  const originals = new Map<string, NoteBlock>();
  function remember(blocks: NoteBlock[]) {
    for (const block of blocks) { originals.set(block.id, block); remember(block.children ?? []); }
  }
  remember(source?.blocks ?? []);
  function equivalent(left: InlineNode[], right: InlineNode[]) {
    const normalize = (nodes: InlineNode[]) => {
      const runs: { type: string; text: string; href?: string; marks: string }[] = [];
      for (const node of nodes) {
        const run = { type: node.type, text: node.text,
          href: node.type === "link" ? node.href : undefined,
          marks: marks.filter((mark) => node.marks?.includes(mark)).join(","),
        };
        const previous = runs.at(-1);
        if (previous && previous.type === run.type && previous.href === run.href && previous.marks === run.marks) previous.text += run.text;
        else if (run.text) runs.push(run);
      }
      return runs;
    };
    return JSON.stringify(normalize(left)) === JSON.stringify(normalize(right));
  }
  function convert(block: EditorBlock): NoteBlock {
    const content: InlineNode[] = [];
    for (const node of block.content) {
      const texts = node.type === "link" ? node.content : [node];
      for (const text of texts) {
        const active = marks.filter((mark) => text.styles[mark]);
        content.push({
          type: node.type,
          text: text.text,
          ...(node.type === "link" ? { href: safeHref(node.href) } : {}),
          ...(active.length ? { marks: active } : {}),
        });
      }
    }
    const original = originals.get(block.id);
    const unchanged = original && equivalent(original.content ?? [], content);
    return {
      ...original,
      id: block.id,
      type: block.type,
      ...(block.type === "heading" ? { level: block.props.level } : {}),
      ...(block.type === "checkListItem" ? { checked: block.props.checked } : {}),
      ...(block.type === "image" ? { asset_id: block.props.assetId } : {}),
      ...(block.type === "codeBlock" && (block.props.language || original?.language !== undefined) ? { language: block.props.language } : {}),
      ...(unchanged ? {} : (content.length || original?.content ? { content } : {})),
      ...(block.children.length || original?.children ? { children: block.children.map(convert) } : {}),
    };
  }
  return { blocks: blocks.map(convert) };
}
