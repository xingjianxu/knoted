import { useCallback, useRef, useState } from "react";
import { Alert, Button, Dialog, Input } from "@bixlab/mtone";
import { conflictCopyTitle, type NoteDraft } from "./autosave";

export interface ConflictDialogProps {
  open: boolean;
  draft: NoteDraft;
  readOnly?: boolean;
  onOpenChange: (open: boolean) => void;
  onLoadServer: () => Promise<unknown>;
  onSaveCopy: (title: string) => Promise<unknown>;
  onCopyDraft: () => Promise<unknown>;
}
/** Note-specific recovery actions; dismissal never changes the editing session. */
export function ConflictDialog({ open, draft, readOnly = false, onOpenChange, onLoadServer, onSaveCopy, onCopyDraft }: ConflictDialogProps) {
  const [title, setTitle] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<string | null>(null);
  const pending = useRef(false);
  const focusError = useCallback((node: HTMLDivElement | null) => { node?.focus(); }, []);
  const copyTitle = title ?? conflictCopyTitle(draft.title);
  async function run(action: () => Promise<unknown>, message: string) {
    if (pending.current) return;
    pending.current = true; setBusy(true); setError(null); setFeedback(null);
    try { await action(); setFeedback(message); }
    catch (failure) { setError(failure instanceof Error ? failure.message : String(failure)); }
    finally { pending.current = false; setBusy(false); }
  }
  return <Dialog open={open} onOpenChange={next => { if (!pending.current) onOpenChange(next); }}
    title="笔记保存冲突" description="本地草稿已保留。">
    <div className="space-y-4">
      {error && <div ref={focusError} tabIndex={-1}><Alert tone="error" title="操作失败">{error}</Alert></div>}
      {feedback && <Alert tone="success" title={feedback} />}
      <Button disabled={busy} onClick={() => void run(onCopyDraft, "已复制本地草稿")}>复制草稿</Button>
      <form className="space-y-2" onSubmit={event => {
        event.preventDefault();
        if (!readOnly) void run(() => onSaveCopy(copyTitle), "已另存副本，原草稿仍保留");
      }}>
        <label className="block">副本标题<Input aria-label="副本标题" value={copyTitle} maxLength={500}
          onChange={event => setTitle(event.target.value)} disabled={busy || readOnly} /></label>
        <Button type="submit" disabled={busy || readOnly}>另存副本</Button>
      </form>
      <section aria-label="服务器版本" className="space-y-2">
        <p>加载服务器版本会替换当前本地草稿。</p>
        <Button danger disabled={busy || readOnly} onClick={() => void run(onLoadServer, "已加载服务器内容")}>加载服务器并替换本地</Button>
      </section>
    </div>
  </Dialog>;
}
