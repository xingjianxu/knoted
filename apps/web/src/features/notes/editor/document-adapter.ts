import type { components } from "@knoted/api-client";

import type { EditorBlock, EditorPartialBlock } from "./schema";

export type NoteDocument = components["schemas"]["NoteDocument"];
type NoteBlock = components["schemas"]["NoteBlock"];
type InlineNode = components["schemas"]["InlineNode"];
type Mark = components["schemas"]["InlineMark"];
const marks: Mark[] = ["bold", "italic", "underline", "strike", "code"];
const blockTypes: NoteBlock["type"][] = ["heading", "paragraph", "bulletListItem", "numberedListItem", "checkListItem", "image", "codeBlock"];

function safeHref(href: string): string {
  // Match the backend's URI (not IRI) spelling contract before WHATWG URL
  // parsing can normalize invalid percent escapes, Unicode or missing slashes.
  if (!/^(?:[A-Za-z0-9\-._~:/?#[\]@!$&'()*+,;=]|%[0-9A-Fa-f]{2})*$/.test(href)
    || href.split("#").length > 2) throw new Error("链接地址无效");
  const url = new URL(href);
  if (url.protocol === "mailto:") {
    if (!url.pathname || /[[\]]/.test(href.slice(href.indexOf(":") + 1))) throw new Error("邮件链接无效");
  } else if (url.protocol === "http:" || url.protocol === "https:") {
    const remainder = href.slice(href.indexOf(":") + 1);
    if (!remainder.startsWith("//") || !url.hostname) throw new Error("链接地址无效");
    const rest = remainder.slice(2);
    const end = rest.search(/[/?#]/);
    const authority = end < 0 ? rest : rest.slice(0, end);
    const suffix = end < 0 ? "" : rest.slice(end);
    const at = authority.lastIndexOf("@");
    const userinfo = at < 0 ? "" : authority.slice(0, at);
    const host = at < 0 ? authority : authority.slice(at + 1);
    if (!authority || /[[\]]/.test(suffix) || /[[\]@]/.test(userinfo)
      || (!host.startsWith("[") && /[[\]]/.test(host))) throw new Error("链接地址无效");
  } else throw new Error("链接协议不受支持");
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
      if (node.href !== undefined) safeHref(node.href);
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
        ...(block.level !== undefined ? { level: block.level } : {}),
        ...(block.checked !== undefined ? { checked: block.checked } : {}),
        ...(block.language !== undefined ? { language: block.language } : {}),
        ...(block.asset_id !== undefined ? { assetId: block.asset_id } : {}),
      },
      content,
      children: (block.children ?? []).map((child) => convert(child, depth + 1)),
    } as EditorPartialBlock;
  }
  return document.blocks.map((block) => convert(block, 1));
}

// Initial sidecar data preserves unchanged inline run boundaries/mark ordering.
// Semantic fields come exclusively from shared editor props, so native history
// and type transitions can never resurrect an obsolete initial value.
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
    if (unchanged) {
      for (const node of original.content ?? []) if (node.href !== undefined) safeHref(node.href);
    }
    const retained = { ...original };
    delete retained.level;
    delete retained.checked;
    delete retained.language;
    delete retained.asset_id;
    return {
      ...retained,
      id: block.id,
      type: block.type,
      ...(block.props.level !== undefined ? { level: block.props.level } : {}),
      ...(block.props.checked !== undefined ? { checked: block.props.checked } : {}),
      ...(block.props.assetId !== undefined ? { asset_id: block.props.assetId } : {}),
      ...(block.props.language !== undefined ? { language: block.props.language } : {}),
      ...(unchanged ? {} : (content.length || original?.content ? { content } : {})),
      ...(block.children.length || original?.children ? { children: block.children.map(convert) } : {}),
    };
  }
  return { blocks: blocks.map(convert) };
}
