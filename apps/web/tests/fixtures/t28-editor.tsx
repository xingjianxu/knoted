import { useState } from "react";

import { NoteEditor, type NoteDocument } from "../../src/features/notes/editor";

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
    <NoteEditor documentKey="t28" document={document} onChange={(value) => { setDocument(value); setChanges((n) => n + 1); }} />
    <output aria-label="保存 JSON">{JSON.stringify(document)}</output>
    <output aria-label="变更次数">{changes}</output>
  </main>;
}
