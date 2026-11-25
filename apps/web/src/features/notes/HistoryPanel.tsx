import { useCallback, useReducer, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Alert, Button, Card, ConfirmDialog, Divider, Empty, List, ListItem, Spinner, Tag } from "@bixlab/mtone";
import { ArrowLeft, RotateCcw, X } from "lucide-react";
import type { QueryClientBundle } from "../../lib/query";
import { queryKeys } from "../../lib/query";
import { createCursorState, cursorReducer } from "../../lib/cursor-state";
import { NoteEditor } from "./editor";
import type { Snapshot } from "./autosave";

const historyDate = new Intl.DateTimeFormat("zh-CN", { dateStyle: "medium", timeStyle: "short", timeZone: "Asia/Shanghai" });
const reasons: Readonly<Record<string, string>> = {
  flush: "手动保存", periodic: "定期保存", recovery: "历史补存", before_restore: "恢复前备份",
};
const snapshotReason = (reason: string) => reasons[reason] ?? reason;

export interface HistoryPanelProps {
  noteId: string;
  bundle: QueryClientBundle;
  disabled?: boolean;
  onClose: () => void;
  onRestore: (snapshot: Snapshot) => Promise<void>;
}

export function HistoryPanel({ noteId, bundle, disabled = false, onClose, onRestore }: HistoryPanelProps) {
  const [page, dispatch] = useReducer(cursorReducer, undefined, () => createCursorState());
  const [selected, setSelected] = useState<Snapshot | null>(null);
  const [confirm, setConfirm] = useState(false);
  const selectedTrigger = useRef<HTMLButtonElement>(null);
  const focusHeading = useCallback((node: HTMLHeadingElement | null) => { node?.focus(); }, []);
  const history = useQuery({
    queryKey: [...queryKeys.notes.snapshots(noteId), page.cursor],
    retry: false,
    queryFn: async ({ signal }) => {
      const { data } = await bundle.api.raw.GET("/notes/{noteId}/snapshots", { signal, params: { path: { noteId }, query: { limit: 50, cursor: page.cursor ?? undefined } } });
      if (!data) throw new Error("无法读取历史版本");
      return data;
    },
  });
  const nextCursor = history.data?.next_cursor ?? null;

  return (
    <section id="note-history" aria-label="笔记历史" className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 ref={focusHeading} tabIndex={-1} className="section-title">历史版本</h2>
        <Button size="sm" variant="outline" onClick={onClose}>
          <X size={14} aria-hidden="true" />
          关闭历史
        </Button>
      </div>
      <div className={selected ? "notes-history-body notes-history--preview" : "notes-history-body"}>
        <Card size="small" className="notes-history-list min-w-0">
          <div className="space-y-3">
            {history.isFetching && <Spinner label="正在读取历史" />}
            {history.error && (
              <Alert tone="error">
                {history.error.message}
                <div className="mt-2">
                  <Button size="sm" onClick={() => void history.refetch()}>重试历史</Button>
                </div>
              </Alert>
            )}
            {history.data?.items.length === 0 && <Empty title="暂无历史版本" />}
            {!!history.data?.items.length && (
              <List aria-label="可预览的版本">
                {history.data.items.map(snapshot => (
                  <ListItem
                    key={snapshot.snapshot_id}
                    description={
                      <span className="inline-flex flex-wrap items-center gap-1.5">
                        <Tag tone="neutral">v{snapshot.content_version}</Tag>
                        <span>{historyDate.format(new Date(snapshot.created_at))}</span>
                        <Tag tone="blue">{snapshotReason(snapshot.reason)}</Tag>
                      </span>
                    }
                  >
                    <Button
                      size="sm"
                      variant={selected?.snapshot_id === snapshot.snapshot_id ? "soft" : "ghost"}
                      className="max-w-full whitespace-normal text-left [overflow-wrap:anywhere]"
                      aria-label={`预览 ${snapshot.title} · v${snapshot.content_version} · ${snapshot.created_at}`}
                      aria-pressed={selected?.snapshot_id === snapshot.snapshot_id}
                      onClick={event => { selectedTrigger.current = event.currentTarget; setSelected(snapshot); }}
                    >
                      {snapshot.title}
                    </Button>
                  </ListItem>
                ))}
              </List>
            )}
            <div className="flex flex-wrap gap-2">
              <Button size="sm" disabled={history.isFetching || page.previous.length === 0} onClick={() => { setSelected(null); dispatch({ type: "previous" }); }}>上一页历史</Button>
              <Button size="sm" disabled={history.isFetching || !nextCursor || nextCursor === page.cursor} onClick={() => { setSelected(null); dispatch({ type: "next", nextCursor }); }}>下一页历史</Button>
            </div>
          </div>
        </Card>

        {selected && (
          <Card size="small" className="notes-history-preview min-w-0">
            <div className="space-y-3">
              <div className="notes-history-back">
                <Button size="sm" variant="ghost" onClick={() => {
                  setSelected(null);
                  requestAnimationFrame(() => selectedTrigger.current?.focus());
                }}>
                  <ArrowLeft size={14} aria-hidden="true" />
                  返回版本列表
                </Button>
              </div>
              <div key={selected.snapshot_id} className="flex flex-wrap items-start justify-between gap-2">
                <div className="min-w-0 space-y-1">
                  <h3 ref={focusHeading} tabIndex={-1} className="section-title">{selected.title}</h3>
                  <div className="flex flex-wrap items-center gap-1.5 text-sm text-muted">
                    <Tag tone="neutral">版本 {selected.content_version}</Tag>
                    <time dateTime={selected.created_at}>{historyDate.format(new Date(selected.created_at))}</time>
                    <Tag tone="blue">{snapshotReason(selected.reason)}</Tag>
                  </div>
                </div>
                <Button size="sm" variant="solid" disabled={disabled} onClick={() => setConfirm(true)}>
                  <RotateCcw size={14} aria-hidden="true" />
                  恢复此历史版本
                </Button>
              </div>
              <Divider />
              <section aria-label="历史预览" className="reading-measure">
                <NoteEditor key={selected.snapshot_id} documentKey={selected.snapshot_id} document={selected.document} readOnly onChange={() => {}} />
              </section>
              <ConfirmDialog
                open={confirm}
                onOpenChange={setConfirm}
                title="恢复历史版本"
                description="先保存当前草稿与历史，再恢复所选版本。"
                confirmText="确认恢复"
                onConfirm={() => onRestore(selected)}
              />
            </div>
          </Card>
        )}
      </div>
    </section>
  );
}
