import { Alert, Button, Dialog, Form, FormField, Input, Select } from "@bixlab/mtone";
import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useRef, useState } from "react";
import { queryKeys, type QueryClientBundle } from "../../lib/query";
import { notesTreeQuery, noteTree } from "../notes/queries";

const root = "root";

function parentOptions(tree: ReturnType<typeof noteTree>, ancestors: string[] = []): Array<{ value: string; label: string }> {
  return tree.flatMap((item) => {
    const title = typeof item.label === "string" && item.label ? item.label : "无标题";
    const path = [...ancestors, title];
    return [{ value: item.id, label: path.join(" / ") }, ...parentOptions(item.children ?? [], path)];
  });
}

export function SaveNoteDialog({ bundle, messageId }: { bundle: QueryClientBundle; messageId: string }) {
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [parent, setParent] = useState(root);
  const [pending, setPending] = useState(false);
  const [problem, setProblem] = useState<string>();
  const [savedNote, setSavedNote] = useState<string>();
  const submitting = useRef(false);
  const keys = useRef(new Map<string, string>());
  const tree = useQuery({ ...notesTreeQuery(bundle), enabled: open });
  const options = [{ value: root, label: "根目录" }, ...parentOptions(noteTree(open ? tree.data ?? [] : []))];
  const validParent = parent === root || (tree.isSuccess && options.some((option) => option.value === parent));

  async function save() {
    const trimmedTitle = title.trim();
    if (!trimmedTitle || !validParent || submitting.current) return;
    const body = { title: trimmedTitle, parent_id: parent === root ? null : parent };
    const identity = JSON.stringify(body);
    const key = keys.current.get(identity) ?? crypto.randomUUID();
    keys.current.set(identity, key);
    submitting.current = true;
    setPending(true);
    setProblem(undefined);
    try {
      const { data, response } = await bundle.api.raw.POST("/chat/messages/{messageId}/save-note", {
        params: { path: { messageId }, header: { "Idempotency-Key": key } },
        body,
      });
      if (!data) {
        if (response.status === 409) void tree.refetch();
        setProblem(response.status === 409 ? "保存失败。请检查回答状态和父页。" : "保存失败，请重试。");
        return;
      }
      setSavedNote(data.note_id);
      setOpen(false);
      void bundle.queryClient.invalidateQueries({ queryKey: queryKeys.notes.list() });
    } catch {
      setProblem("保存失败，请重试。");
    } finally {
      submitting.current = false;
      setPending(false);
    }
  }

  if (savedNote) return <Link className="page-link" to="/notes/$noteId" params={{ noteId: savedNote }}>打开笔记</Link>;
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => { if (!submitting.current) setOpen(next); }}
      title="存为笔记"
      trigger={<Button type="button" variant="ghost">存为笔记</Button>}
    >
      <Form className="space-y-3" onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}>
        <FormField label="标题">
          <Input value={title} onChange={(event) => setTitle(event.target.value)} maxLength={500} required disabled={pending} />
        </FormField>
        <FormField label="父页">
          <Select options={options} value={parent} onValueChange={(value) => setParent(value ?? root)} disabled={pending} />
        </FormField>
        {tree.isError && <Alert tone="error">无法读取笔记目录。{" "}
          <Button type="button" onClick={() => void tree.refetch()}>重试读取</Button>
        </Alert>}
        {problem && <Alert tone="error">{problem}</Alert>}
        <div className="flex justify-end gap-2">
          <Button type="button" disabled={pending} onClick={() => setOpen(false)}>取消</Button>
          <Button type="submit" variant="solid" disabled={pending || !title.trim() || !validParent}>
            {pending ? "正在保存" : "保存"}
          </Button>
        </div>
      </Form>
    </Dialog>
  );
}
