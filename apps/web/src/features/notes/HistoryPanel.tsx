import { useReducer, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Alert, Button, ConfirmDialog, Empty, Spinner } from "@bixlab/mtone";
import type { QueryClientBundle } from "../../lib/query";
import { queryKeys } from "../../lib/query";
import { createCursorState, cursorReducer } from "../../lib/cursor-state";
import { NoteEditor } from "./editor";
import type { Snapshot } from "./autosave";

export interface HistoryPanelProps { noteId: string; bundle: QueryClientBundle; disabled?: boolean; onRestore: (snapshot: Snapshot) => Promise<void>; }
export function HistoryPanel(props: HistoryPanelProps) { return <HistoryContent key={props.noteId} {...props} />; }
function HistoryContent({ noteId, bundle, disabled = false, onRestore }: HistoryPanelProps) {
  const [page, dispatch] = useReducer(cursorReducer, undefined, () => createCursorState());
  const [selected, setSelected] = useState<Snapshot | null>(null);
  const [confirm, setConfirm] = useState(false);
  const history = useQuery({ queryKey: [...queryKeys.notes.snapshots(noteId), page.cursor], retry: false,
    queryFn: async ({ signal }) => {
      const { data } = await bundle.api.raw.GET("/notes/{noteId}/snapshots", { signal, params: { path: { noteId }, query: { limit: 50, cursor: page.cursor ?? undefined } } });
      if (!data) throw new Error("无法读取历史版本"); return data;
    },
  });
  return <section aria-label="笔记历史" className="space-y-3">
    <h2>历史版本</h2>
    {history.isFetching && <Spinner label="正在读取历史" />}
    {history.error && <Alert tone="error">{history.error.message}<Button onClick={() => void history.refetch()}>重试历史</Button></Alert>}
    {history.data?.items.length === 0 && <Empty title="暂无历史版本" />}
    {history.data?.items.map(snapshot => <div key={snapshot.snapshot_id}>
      <Button onClick={() => setSelected(snapshot)}>预览 {snapshot.title} · v{snapshot.content_version} · {snapshot.created_at}</Button>
      <span>{snapshot.reason}</span>
    </div>)}
    <div className="flex gap-2">
      <Button disabled={history.isFetching || page.previous.length === 0} onClick={() => { setSelected(null); dispatch({ type: "previous" }); }}>上一页历史</Button>
      <Button disabled={history.isFetching || history.data?.next_cursor === null || history.data?.next_cursor === undefined} onClick={() => { setSelected(null); dispatch({ type: "next", nextCursor: history.data?.next_cursor ?? null }); }}>下一页历史</Button>
    </div>
    {selected && <>
      <section aria-label="历史预览"><h3>{selected.title}</h3><p>版本 {selected.content_version} · {selected.created_at} · {selected.reason}</p>
        <NoteEditor key={selected.snapshot_id} document={selected.document} readOnly onChange={() => {}} />
      </section>
      <Button disabled={disabled} onClick={() => setConfirm(true)}>恢复此历史版本</Button>
      <ConfirmDialog open={confirm} onOpenChange={setConfirm} title="恢复历史版本" description="先保存当前草稿与历史，再恢复所选版本。" confirmText="确认恢复" onConfirm={async () => { await onRestore(selected); setSelected(null); }} />
    </>}
  </section>;
}
