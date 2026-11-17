import { useState } from "react";

import { NoteEditor, type NoteDocument } from "../../src/features/notes/editor";

export function T27Editor() {
  const [document, setDocument] = useState<NoteDocument>({ blocks: [
    { id: "p", type: "paragraph", content: [{ type: "text", text: "中文输入" }] },
  ] });
  return <main>
    <NoteEditor document={document} onChange={setDocument} />
    <output aria-label="保存 JSON">{JSON.stringify(document)}</output>
  </main>;
}
