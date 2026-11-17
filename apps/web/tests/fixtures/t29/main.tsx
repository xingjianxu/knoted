import { Button } from "@bixlab/mtone";
import { createApiClient } from "@knoted/api-client";
import { useState } from "react";
import { createRoot } from "react-dom/client";
import { ImageBlockControls, CodeBlockControls, NoteEditor, type NoteDocument } from "../../../src/features/notes/editor";
import { SlashMenu, BlockMenu } from "../../../src/features/notes/editor/menus";
import { UiProvider } from "../../../src/lib/ui-provider";
import "../../../src/styles.css";

const client = createApiClient({ baseUrl: "/api/v1" });
const initial: NoteDocument = { blocks: [
  { id: "p", type: "paragraph", content: [{ type: "text", text: "T29 public image fixture" }] },
  { id: "code", type: "codeBlock", language: "custom <dialect>", content: [{ type: "text", text: '<img src=x onerror="boom()"> & <script>alert(1)</script>', marks: ["bold", "code"] }] },
] };
function Fixture() {
  const [source, setSource] = useState(initial);
  const [document, setDocument] = useState(initial);
  const [key, setKey] = useState(0);
  const [changes, setChanges] = useState(0);
  const [noteId, setNoteId] = useState("");
  const [status, setStatus] = useState("");
  async function saveAndReopen() {
    setStatus("保存中");
    try {
      const { data: note } = await client.raw.POST("/notes", { body: { title: "T29 short-lived browser fixture", document } });
      if (!note) throw new Error("note response missing");
      // Expose only the own note identifier for the harness's API cleanup.
      setNoteId(note.id);
      const { data: loaded } = await client.raw.GET("/notes/{noteId}", { params: { path: { noteId: note.id } } });
      if (!loaded) throw new Error("reload response missing");
      setSource(loaded.document); setDocument(loaded.document); setKey((value) => value + 1);
      setStatus("已从真实 GET 重开");
    } catch { setStatus("保存或重开失败"); }
  }
  return <UiProvider><main className="m-6 grid gap-4">
    <NoteEditor key={key} document={source} onChange={(value) => { setDocument(value); setChanges((n) => n + 1); }}>
      <SlashMenu /><BlockMenu /><ImageBlockControls client={client} /><CodeBlockControls />
    </NoteEditor>
    <Button disabled={!!noteId || status === "保存中"} onClick={() => void saveAndReopen()}>保存并重开验收笔记</Button>
    <output aria-label="验收状态">{status}</output><output aria-label="验收笔记 ID">{noteId}</output>
    <output aria-label="变更次数">{changes}</output><pre aria-label="保存 JSON">{JSON.stringify(document)}</pre>
  </main></UiProvider>;
}
createRoot(window.document.getElementById("root")!).render(<Fixture />);
