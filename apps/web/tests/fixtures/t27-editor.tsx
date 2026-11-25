import { useState } from "react";

import { NoteEditor, type NoteDocument } from "../../src/features/notes/editor";

export function T27Editor() {
  const [document, setDocument] = useState<NoteDocument>({ blocks: [
    { id: "p", type: "paragraph", content: [{ type: "text", text: "中文输入" }] },
    { id: "code", type: "codeBlock", language: "rust", content: [{ type: "text", text: "one" }] },
  ] });
  return <main>
    <NoteEditor documentKey="t27" document={document} onChange={setDocument} />
    <output aria-label="保存 JSON">{JSON.stringify(document)}</output>
  </main>;
}
