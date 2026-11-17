import { BlockNoteViewRaw, useCreateBlockNote } from "@blocknote/react";
import { Alert } from "@bixlab/mtone";
import { useEffect, useState, type ReactNode } from "react";

import "@blocknote/core/style.css";
import { fromEditorBlocks, safeHref, toEditorBlocks, type NoteDocument } from "./document-adapter";
import { FormattingToolbar } from "./formatting-toolbar";
import { noteEditorSchema } from "./schema";

export type NoteEditorProps = {
  /** Initial value only. Key the component by note ID to open another note. */
  document: NoteDocument;
  onChange: (document: NoteDocument) => void;
  children?: ReactNode;
  readOnly?: boolean;
};

export function NoteEditor({ document, onChange, children, readOnly = false }: NoteEditorProps) {
  // Deliberately independent of props: parent save/query rerenders must never
  // replace the ProseMirror view, selection or an active IME composition.
  const [initialDocument] = useState(() => document);
  const [inputError, setInputError] = useState("");
  const editor = useCreateBlockNote({
    schema: noteEditorSchema,
    disableExtensions: ["attribution"],
    initialContent: initialDocument.blocks.length ? toEditorBlocks(initialDocument) : undefined,
  }, []);
  useEffect(() => editor.onBeforeChange(({ tr }) => {
    if (!tr.docChanged) return;
    try {
      // A new block can lack its ID until BlockNote's UniqueID append step.
      // Inspect the public transaction's proposed link marks directly: turning
      // it into Block DTOs via getChanges() is unsafe before that step runs.
      tr.doc.descendants(node => {
        for (const mark of node.marks) {
          if (mark.type === editor.pmSchema.marks.link) safeHref(mark.attrs.href);
        }
      });
      setInputError("");
    } catch {
      setInputError("未应用此次编辑：内容含有无法保存的链接或格式，原内容已保留。请修正链接后重试；也可仅以纯文本粘贴链接名称文字。");
      return false;
    }
  }), [editor]);
  return (
    <section aria-label="笔记编辑器">
      {inputError && <Alert tone="warning">{inputError}</Alert>}
      {!readOnly && <FormattingToolbar editor={editor} />}
      <BlockNoteViewRaw
        editor={editor}
        editable={!readOnly}
        onChange={() => { if (!readOnly) onChange(fromEditorBlocks(editor.document, initialDocument)); }}
        formattingToolbar={false}
        linkToolbar={false}
        slashMenu={false}
        sideMenu={false}
        filePanel={false}
        tableHandles={false}
        emojiPicker={false}
        comments={false}
      >{!readOnly && children}</BlockNoteViewRaw>
    </section>
  );
}
