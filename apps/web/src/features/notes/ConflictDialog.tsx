import { useRef, useState } from "react";
import { Alert, Button, Dialog, Input } from "@bixlab/mtone";
import { conflictCopyTitle, type NoteDraft } from "./autosave";

export interface ConflictDialogProps {
  open: boolean;
  draft: NoteDraft;
  onOpenChange: (open: boolean) => void;
  onLoadServer: () => Promise<unknown>;
  onSaveCopy: (title: string) => Promise<unknown>;
  onCopyDraft: () => Promise<unknown>;
}
/** Note-specific recovery actions; dismissal never changes the editing session. */
export function ConflictDialog({ open, draft, onOpenChange, onLoadServer, onSaveCopy, onCopyDraft }: ConflictDialogProps) {
  const [title, setTitle] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<string | null>(null);
  const pending = useRef(false);
  const copyTitle = title ?? conflictCopyTitle(draft.title);
  async function run(action: () => Promise<unknown>, message: string) {
    if (pending.current) return;
    pending.current = true; setBusy(true); setError(null); setFeedback(null);
    try { await action(); setFeedback(message); }
    catch (failure) { setError(failure instanceof Error ? failure.message : String(failure)); }
    finally { pending.current = false; setBusy(false); }
  }
  return <Dialog open={open} onOpenChange={onOpenChange} title="笔记保存冲突" description="本地草稿已保留。加载服务器内容将替换本地编辑内容。">
    <label>副本标题<Input aria-label="副本标题" value={copyTitle} onChange={event => setTitle(event.target.value)} disabled={busy} /></label>
    {error && <Alert tone="error" title="操作失败">{error}</Alert>}
    {feedback && <Alert tone="success" title={feedback} />}
    <div className="flex flex-wrap gap-2">
      <Button disabled={busy} onClick={() => void run(onLoadServer, "已加载服务器内容")}>加载服务器并替换本地</Button>
      <Button disabled={busy} onClick={() => void run(() => onSaveCopy(copyTitle), "已另存副本，原草稿仍保留")}>另存副本</Button>
      <Button disabled={busy} onClick={() => void run(onCopyDraft, "已复制本地草稿")}>复制草稿</Button>
    </div>
  </Dialog>;
}
