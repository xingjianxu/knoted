import type { BlockEditorBlock, BlockEditorInput } from "@bixlab/mtone-blockeditor";
import type { components } from "@knoted/api-client";

export type NoteDocument = components["schemas"]["NoteDocument"];
type NoteBlock = components["schemas"]["NoteBlock"];
type NoteTable = components["schemas"]["NoteTable"];
type InlineNode = components["schemas"]["InlineNode"];
type Mark = components["schemas"]["InlineMark"];

const MAX_BLOCKS = 5000;
const MAX_DEPTH = 8;
const MAX_TABLE_ROWS = 1000;
const MAX_TABLE_COLUMNS = 100;
const MAX_DOCUMENT_CELLS = 10_000;
const MAX_COLUMN_WIDTH = 10_000;
const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const marks: Mark[] = ["bold", "italic", "underline", "strike", "code"];
const blockTypes: NoteBlock["type"][] = [
  "heading",
  "paragraph",
  "bulletListItem",
  "numberedListItem",
  "checkListItem",
  "image",
  "codeBlock",
  "table",
];
const unsupportedBlockLabels: Readonly<Record<string, string>> = {
  quote: "引用",
  divider: "分割线",
  toggleListItem: "折叠块",
  callout: "提示块",
  video: "视频",
  audio: "音频",
  file: "附件",
};

function assetOrigin(): string {
  if (typeof window !== "undefined" && /^https?:\/\//i.test(window.location.origin)) {
    return window.location.origin;
  }
  return "http://localhost";
}

export function isAssetUuid(value: string): boolean {
  return UUID_PATTERN.test(value);
}

/** Build a same-origin HTTP(S) URL accepted by @bixlab/mtone-blockeditor's safeImageUrl check. */
export function assetIdToUrl(assetId: string): string {
  if (!isAssetUuid(assetId)) throw new Error("图片资产 ID 无效");
  return `${assetOrigin()}/api/v1/assets/${encodeURIComponent(assetId)}/content`;
}

/** Extract a persisted Knoted asset UUID from a same-origin asset content URL. */
export function urlToAssetId(rawUrl: string): string | null {
  const trimmed = rawUrl.trim();
  if (!trimmed) return null;
  let pathname: string;
  if (trimmed.startsWith("/")) {
    if (trimmed.includes("?") || trimmed.includes("#")) return null;
    pathname = trimmed;
  } else {
    let parsed: URL;
    try {
      parsed = new URL(trimmed);
    } catch {
      return null;
    }
    if (
      parsed.origin !== assetOrigin()
      || parsed.username
      || parsed.password
      || parsed.search
      || parsed.hash
    ) {
      return null;
    }
    pathname = parsed.pathname;
  }
  const match = pathname.match(/^\/api\/v1\/assets\/([^/]+)\/content$/);
  if (!match) return null;
  let decoded: string;
  try {
    decoded = decodeURIComponent(match[1]!);
  } catch {
    return null;
  }
  return isAssetUuid(decoded) ? decoded : null;
}

export class NoteLinkError extends Error {}

export function safeHref(href: string): string {
  // Match the backend's URI (not IRI) spelling contract before WHATWG URL
  // parsing can normalize invalid percent escapes, Unicode or missing slashes.
  if (!/^(?:[A-Za-z0-9\-._~:/?#[\]@!$&'()*+,;=]|%[0-9A-Fa-f]{2})*$/.test(href)
    || href.split("#").length > 2) throw new NoteLinkError("链接地址无效");
  let url: URL;
  try {
    url = new URL(href);
  } catch {
    throw new NoteLinkError("链接地址无效");
  }
  if (url.protocol === "mailto:") {
    if (!url.pathname || /[[\]]/.test(href.slice(href.indexOf(":") + 1))) throw new NoteLinkError("邮件链接无效");
  } else if (url.protocol === "http:" || url.protocol === "https:") {
    const remainder = href.slice(href.indexOf(":") + 1);
    if (!remainder.startsWith("//") || !url.hostname) throw new NoteLinkError("链接地址无效");
    const rest = remainder.slice(2);
    const end = rest.search(/[/?#]/);
    const authority = end < 0 ? rest : rest.slice(0, end);
    const suffix = end < 0 ? "" : rest.slice(end);
    const at = authority.lastIndexOf("@");
    const userinfo = at < 0 ? "" : authority.slice(0, at);
    const host = at < 0 ? authority : authority.slice(at + 1);
    if (!authority || /[[\]]/.test(suffix) || /[[\]@]/.test(userinfo)
      || (!host.startsWith("[") && /[[\]]/.test(host))) throw new NoteLinkError("链接地址无效");
  } else throw new NoteLinkError("链接协议不受支持");
  return href;
}

function plainText(nodes: readonly InlineNode[] | undefined): string {
  return (nodes ?? []).map((node) => node.text).join("");
}

function validateTable(table: NoteTable, previousCells: number): number {
  const rows = table.rows.length;
  const columns = table.rows[0]?.cells.length ?? 0;
  if (!columns || table.rows.some(row => row.cells.length !== columns)) {
    throw new Error("表格至少需要一行一列，且每行的列数必须相同");
  }
  const cells = previousCells + rows * columns;
  if (rows > MAX_TABLE_ROWS || columns > MAX_TABLE_COLUMNS || cells > MAX_DOCUMENT_CELLS) {
    throw new Error("表格超出保存限制（每表最多 1000 行、100 列，每篇笔记最多 10000 个单元格）");
  }
  const headerRows = table.header_rows ?? 0;
  const headerColumns = table.header_columns ?? 0;
  if (!Number.isInteger(headerRows) || headerRows < 0 || headerRows > rows
    || !Number.isInteger(headerColumns) || headerColumns < 0 || headerColumns > columns) {
    throw new Error("表格的表头超出行列范围");
  }
  const widths = table.column_widths;
  if (widths && (widths.length !== columns || widths.some(width =>
    width !== null && (!Number.isInteger(width) || width < 1 || width > MAX_COLUMN_WIDTH)))) {
    throw new Error("表格列宽无效");
  }
  return cells;
}

function toEditorInline(nodes: readonly InlineNode[]) {
  return nodes.map((node) => {
    if (node.type !== "text" && node.type !== "link") throw new Error("不支持的行内格式");
    if (node.href !== undefined) safeHref(node.href);
    const styles: Partial<Record<Mark, boolean>> = {};
    for (const mark of node.marks ?? []) {
      if (!marks.includes(mark)) throw new Error("不支持的文字格式");
      styles[mark] = true;
    }
    // BlockEditor's code mark excludes links and all other marks.
    if (node.type === "link") {
      delete styles.code;
      return {
        type: "link" as const,
        href: safeHref(node.href ?? ""),
        content: [{ type: "text" as const, text: node.text, styles }],
      };
    }
    return { type: "text" as const, text: node.text, styles: styles.code ? { code: true } : styles };
  });
}

/** Explicit DTO boundary: BlockEditor props/styles/URLs never reach HTTP. */
export function toEditorBlocks(document: NoteDocument): BlockEditorInput[] {
  const ids = new Set<string>();
  let count = 0;
  let cells = 0;

  function convert(block: NoteBlock, depth: number): BlockEditorInput {
    if (!blockTypes.includes(block.type)) throw new Error("不支持的笔记块");
    if (!block.id || [...block.id].length > 64 || ids.has(block.id)) throw new Error("笔记块 ID 无效");
    ids.add(block.id);
    if (++count > MAX_BLOCKS || depth > MAX_DEPTH) throw new Error("笔记块超出限制");
    if (block.type === "heading" && ![1, 2, 3].includes(block.level ?? 0)) throw new Error("标题级别无效");
    if (block.type === "image" && (!block.asset_id || !isAssetUuid(block.asset_id))) {
      throw new Error("图片缺少有效资产 ID");
    }

    if (block.type !== "table" && block.table !== undefined) throw new Error("表格块类型无效");
    const content = toEditorInline(block.content ?? []);

    const children = (block.children ?? []).map((child) => convert(child, depth + 1));

    switch (block.type) {
      case "table": {
        if (!block.table || content.length || block.level !== undefined || block.checked !== undefined
          || block.asset_id !== undefined || block.language !== undefined) {
          throw new Error("表格块格式无效");
        }
        const table = block.table;
        cells = validateTable(table, cells);
        return {
          id: block.id,
          type: "table",
          content: {
            type: "tableContent",
            rows: table.rows.map(row => ({ cells: row.cells.map(toEditorInline) })),
            columnWidths: table.column_widths?.map(width => width ?? undefined) ?? [],
            headerRows: table.header_rows ?? 0,
            headerCols: table.header_columns ?? 0,
          },
          children,
        };
      }
      case "heading":
        return {
          id: block.id,
          type: "heading",
          props: { level: block.level as 1 | 2 | 3 },
          content,
          children,
        };
      case "checkListItem":
        return {
          id: block.id,
          type: "checkListItem",
          props: { checked: block.checked ?? false },
          content,
          children,
        };
      case "codeBlock": {
        const text = plainText(block.content);
        return {
          id: block.id,
          type: "codeBlock",
          props: { language: block.language ? block.language : "text" },
          ...(text ? { content: text } : {}),
          children,
        };
      }
      case "image":
        return {
          id: block.id,
          type: "image",
          props: {
            url: assetIdToUrl(block.asset_id!),
            alt: "笔记图片",
            caption: plainText(block.content),
          },
          children,
        };
      case "paragraph":
      case "bulletListItem":
      case "numberedListItem":
        return {
          id: block.id,
          type: block.type,
          content,
          children,
        };
    }
  }

  return document.blocks.map((block) => convert(block, 1));
}

// Initial sidecar data preserves unchanged inline run boundaries/mark ordering
// and untouched block metadata while enforcing the NoteDocument v1 allowlist.
export function fromEditorBlocks(
  blocks: readonly BlockEditorBlock[],
  source?: NoteDocument,
): NoteDocument {
  const originals = new Map<string, NoteBlock>();
  function remember(list: readonly NoteBlock[]) {
    for (const block of list) {
      originals.set(block.id, block);
      remember(block.children ?? []);
    }
  }
  remember(source?.blocks ?? []);

  function equivalent(left: readonly InlineNode[], right: readonly InlineNode[]) {
    const normalize = (nodes: readonly InlineNode[]) => {
      const runs: { type: string; text: string; href?: string; marks: string }[] = [];
      for (const node of nodes) {
        const activeMarks = marks.filter((mark) => node.marks?.includes(mark));
        const normalizedMarks = node.type === "link"
          ? activeMarks.filter((mark) => mark !== "code")
          : activeMarks.includes("code")
            ? ["code"]
            : activeMarks;
        const run = {
          type: node.type,
          text: node.text,
          href: node.type === "link" ? node.href : undefined,
          marks: normalizedMarks.join(","),
        };
        const previous = runs.at(-1);
        if (previous && previous.type === run.type && previous.href === run.href && previous.marks === run.marks) {
          previous.text += run.text;
        } else if (run.text) {
          runs.push(run);
        }
      }
      return runs;
    };
    return JSON.stringify(normalize(left)) === JSON.stringify(normalize(right));
  }

  function validateOriginalHrefs(nodes: readonly InlineNode[] | undefined) {
    for (const node of nodes ?? []) {
      if (node.href !== undefined) safeHref(node.href);
    }
  }

  const seenIds = new Set<string>();
  let count = 0;
  let cells = 0;

  function convert(block: BlockEditorBlock, depth: number): NoteBlock {
    if (++count > MAX_BLOCKS || depth > MAX_DEPTH) {
      throw new Error("笔记块数量或嵌套层级超出保存限制（最多 5000 块、8 层嵌套）");
    }
    if (!block.id || [...block.id].length > 64 || seenIds.has(block.id)) {
      throw new Error("笔记块 ID 无效");
    }
    seenIds.add(block.id);

    const original = originals.get(block.id);
    const sameType = original?.type === block.type;
    const retained: Partial<NoteBlock> = sameType ? { ...original } : {};
    delete retained.level;
    delete retained.checked;
    delete retained.language;
    delete retained.asset_id;
    delete retained.content;
    delete retained.table;
    delete retained.children;

    const children = block.children.map((child) => convert(child, depth + 1));
    const childrenField = children.length || (sameType && original?.children) ? { children } : {};

    switch (block.type) {
      case "table": {
        const previous = original?.table;
        const rows = block.content.rows.map((row, rowIndex) => ({
          cells: row.cells.map((cell, column) => {
            if (!Array.isArray(cell)
              && ((cell.props.colspan ?? 1) !== 1 || (cell.props.rowspan ?? 1) !== 1)) {
              throw new Error("暂不支持保存带合并单元格的表格，请拆分单元格后重试");
            }
            const content = convertInlineContent(Array.isArray(cell) ? cell : cell.content);
            const prior = previous?.rows[rowIndex]?.cells[column];
            if (prior && equivalent(prior, content)) {
              validateOriginalHrefs(prior);
              return prior;
            }
            return content;
          }),
        }));
        const widths = (rows[0]?.cells ?? []).map((_, column) => block.content.columnWidths[column] ?? null);
        const headerRows = block.content.headerRows ?? 0;
        const headerColumns = block.content.headerCols ?? 0;
        const table: NoteTable = {
          rows,
          ...(headerRows || previous?.header_rows !== undefined ? { header_rows: headerRows } : {}),
          ...(headerColumns || previous?.header_columns !== undefined ? { header_columns: headerColumns } : {}),
          ...(widths.some(width => width !== null) || previous?.column_widths ? { column_widths: widths } : {}),
        };
        cells = validateTable(table, cells);
        return { id: block.id, type: "table", table, ...childrenField };
      }
      case "heading": {
        if (![1, 2, 3].includes(block.props.level)) {
          throw new Error("暂不支持保存 4–6 级标题，请调整为 1–3 级标题后再保存");
        }
        const content = convertInlineContent(block.content);
        const unchanged = sameType && equivalent(original?.content ?? [], content);
        if (unchanged) validateOriginalHrefs(original?.content);
        return {
          ...retained,
          id: block.id,
          type: "heading",
          level: block.props.level as 1 | 2 | 3,
          ...(unchanged
            ? (original?.content ? { content: original.content } : {})
            : (content.length || (sameType && original?.content) ? { content } : {})),
          ...childrenField,
        };
      }
      case "paragraph":
      case "bulletListItem":
      case "numberedListItem": {
        const content = convertInlineContent(block.content);
        const unchanged = sameType && equivalent(original?.content ?? [], content);
        if (unchanged) validateOriginalHrefs(original?.content);
        return {
          ...retained,
          id: block.id,
          type: block.type,
          ...(unchanged
            ? (original?.content ? { content: original.content } : {})
            : (content.length || (sameType && original?.content) ? { content } : {})),
          ...childrenField,
        };
      }
      case "checkListItem": {
        const content = convertInlineContent(block.content);
        const unchanged = sameType && equivalent(original?.content ?? [], content);
        if (unchanged) validateOriginalHrefs(original?.content);
        return {
          ...retained,
          id: block.id,
          type: "checkListItem",
          checked: Boolean(block.props.checked),
          ...(unchanged
            ? (original?.content ? { content: original.content } : {})
            : (content.length || (sameType && original?.content) ? { content } : {})),
          ...childrenField,
        };
      }
      case "codeBlock": {
        const text = block.content.map((node) => node.text).join("");
        const unchanged = sameType && plainText(original?.content) === text;
        if (unchanged) validateOriginalHrefs(original?.content);
        const rawLanguage = block.props.language;
        let language: string | undefined;
        if (sameType && rawLanguage === "text" && (original?.language === undefined || original.language === "" || original.language === "text")) {
          language = original?.language;
        } else {
          language = rawLanguage === "text" ? "" : rawLanguage;
        }
        const content: InlineNode[] = text ? [{ type: "text", text }] : [];
        return {
          ...retained,
          id: block.id,
          type: "codeBlock",
          ...(language !== undefined ? { language } : {}),
          ...(unchanged
            ? (original?.content ? { content: original.content } : {})
            : (content.length || (sameType && original?.content) ? { content } : {})),
          ...childrenField,
        };
      }
      case "image": {
        const assetId = urlToAssetId(block.props.url);
        if (!assetId) {
          throw new Error("图片需通过“上传图片”上传后才能保存，暂不支持直接保存外部图片地址");
        }
        const caption = block.props.caption ?? "";
        const unchanged = sameType && plainText(original?.content) === caption;
        if (unchanged) validateOriginalHrefs(original?.content);
        const content: InlineNode[] = caption ? [{ type: "text", text: caption }] : [];
        return {
          ...retained,
          id: block.id,
          type: "image",
          asset_id: assetId,
          ...(unchanged
            ? (original?.content ? { content: original.content } : {})
            : (content.length || (sameType && original?.content) ? { content } : {})),
          ...childrenField,
        };
      }
      default: {
        const label = unsupportedBlockLabels[block.type] ?? block.type;
        throw new Error(`当前笔记格式暂不支持保存「${label}」块，请移除或转换为支持的块类型后再保存`);
      }
    }
  }

  function convertInlineContent(
    nodes: readonly (
      | { type: "text"; text: string; styles: Partial<Record<string, unknown>> }
      | { type: "link"; href: string; content: readonly { type: "text"; text: string; styles: Partial<Record<string, unknown>> }[] }
    )[],
  ): InlineNode[] {
    const content: InlineNode[] = [];
    for (const node of nodes) {
      const texts = node.type === "link" ? node.content : [node];
      for (const text of texts) {
        const active = marks.filter((mark) => Boolean(text.styles[mark]));
        content.push({
          type: node.type,
          text: text.text,
          ...(node.type === "link" ? { href: safeHref(node.href) } : {}),
          ...(active.length ? { marks: active } : {}),
        });
      }
    }
    return content;
  }

  return { blocks: blocks.map((block) => convert(block, 1)) };
}
