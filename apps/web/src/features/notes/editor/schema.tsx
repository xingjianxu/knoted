import {
  BlockNoteSchema,
  createHeadingBlockSpec,
  defaultBlockSpecs,
  defaultStyleSpecs,
} from "@blocknote/core";
import { createReactBlockSpec } from "@blocknote/react";

// Both persisted block types allow styled inline content. The upstream image
// uses URL-only props and its code block is plain text, so neither is lossless.
const image = createReactBlockSpec(
  { type: "image", propSchema: { assetId: { default: "" } }, content: "inline" },
  {
    render: ({ block, contentRef }) => (
      <figure data-asset-id={block.props.assetId}>
        <span contentEditable={false}>图片</span>
        <figcaption ref={contentRef} />
      </figure>
    ),
  },
)();
const codeBlock = createReactBlockSpec(
  { type: "codeBlock", propSchema: { language: { default: "" } }, content: "inline" },
  { render: ({ contentRef }) => <pre><code ref={contentRef} /></pre> },
)();

// The API permits code together with every other mark. Keep upstream parse
// rules and commands while removing Tiptap's mutually exclusive code default.
const codeStyle: typeof defaultStyleSpecs.code = {
  ...defaultStyleSpecs.code,
  implementation: {
    ...defaultStyleSpecs.code.implementation,
    mark: defaultStyleSpecs.code.implementation.mark.extend({ excludes: "" }),
  },
};

export const noteEditorSchema = BlockNoteSchema.create({
  blockSpecs: {
    paragraph: defaultBlockSpecs.paragraph,
    heading: createHeadingBlockSpec({ levels: [1, 2, 3] }),
    bulletListItem: defaultBlockSpecs.bulletListItem,
    numberedListItem: defaultBlockSpecs.numberedListItem,
    checkListItem: defaultBlockSpecs.checkListItem,
    image,
    codeBlock,
  },
  styleSpecs: {
    bold: defaultStyleSpecs.bold,
    italic: defaultStyleSpecs.italic,
    underline: defaultStyleSpecs.underline,
    strike: defaultStyleSpecs.strike,
    code: codeStyle,
  },
});
export type NoteEditorInstance = typeof noteEditorSchema.BlockNoteEditor;
export type EditorBlock = typeof noteEditorSchema.Block;
export type EditorPartialBlock = typeof noteEditorSchema.PartialBlock;
