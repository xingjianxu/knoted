import { useEffect, useState } from "react";
import { useBlockNoteEditor } from "@blocknote/react";

import { NoteEditor, type NoteDocument } from "../../src/features/notes/editor";

function EditorSelectionProbe() {
  const editor = useBlockNoteEditor();
  // Test-only read access lets browser checks await actual editor selection
  // after a pointer click, without programmatically setting the caret.
  useEffect(() => {
    Object.assign(window, { t27CursorBlock: () => editor.getTextCursorPosition().block.type });
    return () => { Reflect.deleteProperty(window, "t27CursorBlock"); };
  }, [editor]);
  return null;
}
export function T27Editor() {
  const [document, setDocument] = useState<NoteDocument>({ blocks: [
    { id: "p", type: "paragraph", content: [{ type: "text", text: "中文输入" }] },
    { id: "code", type: "codeBlock", language: "rust", content: [{ type: "text", text: "one" }] },
  ] });
  return <main>
    <NoteEditor document={document} onChange={setDocument}><EditorSelectionProbe /></NoteEditor>
    <output aria-label="保存 JSON">{JSON.stringify(document)}</output>
  </main>;
}
