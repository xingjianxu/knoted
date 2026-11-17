import {
  BlockNoteSchema,
  type BlockSpec,
  type BlockImplementation,
  type PropSchema,
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
  // Reuse the published code keyboard extension (Enter, Tab, Shift-Enter,
  // triple-Enter exit), which is independent of plain vs styled content.
  defaultBlockSpecs.codeBlock.extensions,
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

// Every DTO optional semantic field is a real, shared editor prop. BlockNote
// carries shared props across type changes, and its history restores them with
// the document, so a former type cannot resurrect initial-sidecar values.
const semanticProps = {
  level: { default: undefined, type: "number", optional: true },
  checked: { default: undefined, type: "boolean", optional: true },
  language: { default: undefined, type: "string", optional: true },
  assetId: { default: undefined, type: "string", optional: true },
} as const;
function withSemanticProps<const T extends string, const P extends PropSchema, const C extends "inline" | "none" | "plain">(spec: BlockSpec<T, P, C>) {
  type CombinedProps = Omit<typeof semanticProps, keyof P> & P;
  const propSchema = { ...semanticProps, ...spec.config.propSchema } as CombinedProps;
  // Return a raw spec: BlockNoteSchema must resolve the React render and
  // keyboard implementation exactly once when it builds the actual editor.
  const extended: BlockSpec<T, CombinedProps, C> = {
    ...spec,
    config: { ...spec.config, propSchema },
    // Every original prop remains unchanged. The invariant editor generic in
    // the public implementation type does not model additive shared props.
    implementation: spec.implementation as unknown as BlockImplementation<T, CombinedProps, C>,
  };
  return extended;
}

export const noteEditorSchema = BlockNoteSchema.create({
  blockSpecs: {
    paragraph: withSemanticProps(defaultBlockSpecs.paragraph),
    heading: withSemanticProps(createHeadingBlockSpec({ levels: [1, 2, 3] })),
    bulletListItem: withSemanticProps(defaultBlockSpecs.bulletListItem),
    numberedListItem: withSemanticProps(defaultBlockSpecs.numberedListItem),
    checkListItem: withSemanticProps(defaultBlockSpecs.checkListItem),
    image: withSemanticProps(image),
    codeBlock: withSemanticProps(codeBlock),
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
