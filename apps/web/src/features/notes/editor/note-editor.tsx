import { BlockNoteViewRaw, useCreateBlockNote } from "@blocknote/react";
import { useState, type ReactNode } from "react";

import "@blocknote/core/style.css";
import { fromEditorBlocks, toEditorBlocks, type NoteDocument } from "./document-adapter";
import { FormattingToolbar } from "./formatting-toolbar";
import { noteEditorSchema } from "./schema";

export type NoteEditorProps = {
  /** Initial value only. Key the component by note ID to open another note. */
  document: NoteDocument;
  onChange: (document: NoteDocument) => void;
  children?: ReactNode;
};

export function NoteEditor({ document, onChange, children }: NoteEditorProps) {
  // Deliberately independent of props: parent save/query rerenders must never
  // replace the ProseMirror view, selection or an active IME composition.
  const [initialDocument] = useState(() => document);
  const editor = useCreateBlockNote({
    schema: noteEditorSchema,
    disableExtensions: ["attribution"],
    initialContent: initialDocument.blocks.length ? toEditorBlocks(initialDocument) : undefined,
  }, []);
  return (
    <section aria-label="笔记编辑器">
      <FormattingToolbar editor={editor} />
      <BlockNoteViewRaw
        editor={editor}
        onChange={() => onChange(fromEditorBlocks(editor.document, initialDocument))}
        formattingToolbar={false}
        linkToolbar={false}
        slashMenu={false}
        sideMenu={false}
        filePanel={false}
        tableHandles={false}
        emojiPicker={false}
        comments={false}
      >{children}</BlockNoteViewRaw>
    </section>
  );
}
