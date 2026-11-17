import { useEffect, useState } from "react";
import { useBlockNoteEditor } from "@blocknote/react";

import { NoteEditor, noteEditorSchema, type NoteDocument } from "../../src/features/notes/editor";
import { BlockMenu, SlashMenu } from "../../src/features/notes/editor/menus";

function Probe({ reset }: { reset: () => void }) {
  const editor = useBlockNoteEditor(noteEditorSchema);
  useEffect(() => {
    Object.assign(window, { t28Editor: editor, t28ResetChanges: reset });
    return () => { Reflect.deleteProperty(window, "t28Editor"); Reflect.deleteProperty(window, "t28ResetChanges"); };
  }, [editor, reset]);
  return null;
}
export function T28Editor() {
  const [document, setDocument] = useState<NoteDocument>({ blocks: [
    { id: "p", type: "paragraph" },
    { id: "q", type: "paragraph", content: [{ type: "text", text: "first selection" }] },
    { id: "r", type: "paragraph", content: [{ type: "text", text: "last" }], children: [
      { id: "child", type: "bulletListItem", content: [{ type: "text", text: "nested" }] },
    ] },
  ] });
  const [changes, setChanges] = useState(0);
  return <main className="mx-auto max-w-3xl p-12">
    <NoteEditor document={document} onChange={(value) => { setDocument(value); setChanges((n) => n + 1); }}>
      <SlashMenu /><BlockMenu /><Probe reset={() => setChanges(0)} />
    </NoteEditor>
    <output aria-label="保存 JSON">{JSON.stringify(document)}</output>
    <output aria-label="变更次数">{changes}</output>
  </main>;
}
