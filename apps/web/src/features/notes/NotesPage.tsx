import { useCallback, useContext, useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useBlocker, useNavigate, useRouterState } from "@tanstack/react-router";
import { Alert, Button, ConfirmDialog, Dialog, Drawer, Empty, Input, Menu, Select, Spinner, Tree } from "@bixlab/mtone";
import { ApiRequestError } from "@knoted/api-client";
import type { QueryClientBundle } from "../../lib/query";
import { queryKeys } from "../../lib/query";
import { createNoteDraftStore, useNoteAutosave, type Note, type NoteDraftStore, type NoteEditingSession } from "./autosave";
import { ConflictDialog } from "./ConflictDialog";
import { HistoryPanel } from "./HistoryPanel";
import { NoteEditor } from "./editor";
import { SlashMenu, BlockMenu } from "./editor/menus";
import { ImageBlockControls, CodeBlockControls } from "./editor/blocks";
import { NoteAssociations } from "./links/NoteAssociations";
import { NotesNavigationBlocked } from "./navigation";
import { descendants, noteQuery, notesTreeQuery, noteTree } from "./queries";
import "./notes-workspace.css";

interface DeleteReceipt { id: string; title: string; originalParent: string | null; expectedVersion: number; }
const message = (cause: unknown) => cause instanceof Error ? cause.message : "操作失败，请重试。";
const ROOT = "root";
function createNoteWriteAccess() {
  let expired = false;
  const deleted = new Set<string>();
  return {
    expire: () => { expired = true; }, resume: () => { expired = false; },
    isExpired: () => expired, isDeleted: (id: string) => deleted.has(id),
    markDeleted: (id: string) => { deleted.add(id); }, markRestored: (id: string) => { deleted.delete(id); },
    assertCanWrite: (id: string) => {
      if (expired) throw new Error("会话已失效，草稿已保留；重新登录后请明确重试。");
      if (deleted.has(id)) throw new Error("笔记已删除，请先恢复。");
    },
  };
}

/** The notes parent route owns local sessions across note changes; Query only owns server data. */
export function NotesPage({ bundle }: { bundle: QueryClientBundle }) {
  const navigate = useNavigate();
  const notifyBlocked = useContext(NotesNavigationBlocked);
  const pathname = useRouterState({ select: state => state.location.pathname });
  // UUID identity is case-insensitive; server DTOs and retained sessions use lowercase.
  const noteId = pathname.match(/^\/notes\/([0-9a-f-]+)\/?$/i)?.[1]?.toLowerCase();
  const [writeAccess] = useState(createNoteWriteAccess);
  const [expired, setExpired] = useState(false);
  const assertCanWrite = writeAccess.assertCanWrite;
  const [draftStore] = useState(() => createNoteDraftStore({ ...bundle, assertCanWrite }));
  const [opened, setOpened] = useState<Record<string, Note>>({});
  const active = useRef<NoteEditingSession | null>(null);
  const setActiveSession = useCallback((session: NoteEditingSession | null) => { active.current = session; }, []);
  const busy = useRef(false);
  const bypass = useRef<string | null>(null);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState("");
  const [expanded, setExpanded] = useState<string[]>([]);
  const [directorySelection, setDirectorySelection] = useState<string>();
  const [openDirectoryFor, setOpenDirectoryFor] = useState<string>();
  const directoryTrigger = useRef<HTMLDivElement>(null);
  const directoryHeading = useRef<HTMLHeadingElement>(null);
  const [receipts, setReceipts] = useState<DeleteReceipt[]>([]);
  const [rootRecovery, setRootRecovery] = useState<string | null>(null);
  useEffect(() => {
    const middleware = { onResponse: ({ response }: { response: Response }) => {
      if (response.status === 401) { writeAccess.expire(); setExpired(true); }
      return response;
    } };
    bundle.api.raw.use(middleware);
    return () => { bundle.api.raw.eject(middleware); };
  }, [bundle, writeAccess]);
  // The opener stays mounted as a completed navigation closes the drawer.
  if (openDirectoryFor && openDirectoryFor !== noteId) setOpenDirectoryFor(undefined);
  useEffect(() => {
    const trigger = directoryTrigger.current;
    if (!openDirectoryFor || !trigger || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      if (trigger.getClientRects().length > 0) return;
      setOpenDirectoryFor(undefined);
      const heading = directoryHeading.current;
      window.requestAnimationFrame(() => { if (heading?.isConnected) heading.focus(); });
    });
    observer.observe(trigger);
    return () => observer.disconnect();
  }, [openDirectoryFor]);
  const tree = useQuery(notesTreeQuery(bundle));
  const detail = useQuery({ ...noteQuery(bundle, noteId ?? ""), enabled: Boolean(noteId) && !writeAccess.isDeleted(noteId!) });
  // Titles can use confirmed detail responses immediately. Parent/position stay
  // owned by the complete directory read; local drafts never enter this view.
  const directory = tree.data?.map(note => {
    const confirmed = bundle.queryClient.getQueryData<Note>(queryKeys.notes.detail(note.id));
    return confirmed && confirmed.content_version >= note.content_version ? { ...note, title: confirmed.title } : note;
  });
  // Reveal a direct-linked child once its complete directory becomes available.
  if (directory && directorySelection !== noteId) {
    const current = directory.find(note => note.id === noteId);
    if (!noteId || current) {
      setDirectorySelection(noteId);
      const ancestors: string[] = [];
      let parentId = current?.parent_id;
      while (parentId) {
        ancestors.push(parentId);
        parentId = directory.find(note => note.id === parentId)?.parent_id;
      }
      if (ancestors.length) setExpanded([...new Set([...expanded, ...ancestors])]);
    }
  }
  // Retain the first confirmed DTO for each mounted local session, including
  // across server-cache eviction. Later Query data only indicates remote change.
  if (detail.data && !opened[detail.data.id]) {
    setOpened({ ...opened, [detail.data.id]: detail.data });
  }
  const initialNote = noteId ? opened[noteId] ?? detail.data : undefined;
  useEffect(() => {
    if (!initialNote && Object.keys(opened).length === 0 && (expired || detail.error instanceof ApiRequestError && detail.error.status === 401)) {
      void navigate({ to: "/login", search: { redirect: noteId ? `/notes/${noteId}` : "/notes" }, replace: true });
    }
  }, [detail.error, initialNote, navigate, noteId, opened, expired]);
  useBlocker({ enableBeforeUnload: false, shouldBlockFn: async ({ current, next }) => {
    if (current.pathname === next.pathname) return false;
    if (bypass.current === next.pathname) { bypass.current = null; return false; }
    if (busy.current) { setError("操作尚未完成，暂时无法离开。"); notifyBlocked(); return true; }
    if (!active.current) return false;
    try { await active.current.flush(); return false; }
    catch (cause) { setError(`无法离开：${message(cause)} 本地草稿仍保留。`); notifyBlocked(); return true; }
  } });
  function setBusy(value: boolean) { busy.current = value; setWorking(value); }
  async function refreshTree() {
    await bundle.queryClient.invalidateQueries({ queryKey: notesTreeQuery(bundle).queryKey });
  }
  async function run(operation: () => Promise<void>) {
    if (busy.current || writeAccess.isExpired()) return;
    setBusy(true); setError("");
    try { await operation(); } catch (cause) { setError(message(cause)); } finally { setBusy(false); }
  }
  async function create(parent: string | null) {
    // A create can succeed while the user keeps a failed source draft. Navigation still uses the same blocker.
    await run(async () => {
      const { data } = await bundle.api.raw.POST("/notes", { body: { title: "未命名笔记", document: { blocks: [] }, parent_id: parent } });
      if (!data) throw new Error("创建响应缺少笔记");
      bundle.queryClient.setQueryData(queryKeys.notes.detail(data.id), data);
      await refreshTree(); setBusy(false);
      await navigate({ to: "/notes/$noteId", params: { noteId: data.id } });
    });
  }
  async function undo(receipt: DeleteReceipt, root = false) {
    await run(async () => {
      try {
        const { data } = await bundle.api.raw.POST("/notes/{noteId}/restore", { params: { path: { noteId: receipt.id } }, body: { expected_version: receipt.expectedVersion, ...(root ? { restore_parent: { parent_id: null } } : {}) } });
        if (!data) throw new Error("恢复响应缺少笔记");
        writeAccess.markRestored(data.id); draftStore.forget(data.id);
        setOpened(previous => { const next = { ...previous }; delete next[data.id]; return next; });
        bundle.queryClient.setQueryData(queryKeys.notes.detail(data.id), data);
        setReceipts(previous => previous.filter(item => item.id !== data.id)); setRootRecovery(null);
        await refreshTree(); setBusy(false);
        await navigate({ to: "/notes/$noteId", params: { noteId: data.id } });
      } catch (cause) {
        if (cause instanceof ApiRequestError && cause.status === 409 && !root) setRootRecovery(receipt.id);
        throw cause;
      }
    });
  }
  const directoryContents = <>
    <Button size="sm" disabled={working || expired} onClick={() => void create(null)}>新建根笔记</Button>
    {tree.isFetching && <Spinner label="正在读取完整笔记树" />}
    {tree.error && <Alert tone="error">目录读取失败：{tree.error.message}<Button size="sm" onClick={() => void tree.refetch()}>重试目录</Button></Alert>}
    {directory && directory.length > 0 && <Tree className="notes-tree" label="笔记树" items={noteTree(directory)}
      value={noteId} expanded={expanded} onExpandedChange={setExpanded}
      onValueChange={id => { void navigate({ to: "/notes/$noteId", params: { noteId: id } }); }} />}
    {directory?.length === 0 && !tree.isFetching && !tree.error && <Empty title="暂无笔记" />}
  </>;
  return <div className="space-y-4">
    {error && <Alert tone="error">{error}</Alert>}
    {expired && <Alert tone="warning" title="会话已失效，本地草稿已保留">
      <p>远程写入已停止。请先复制草稿，或在新标签页重新登录，再明确重试原版本。</p>
      <a href={`/login?redirect=${encodeURIComponent(pathname)}`} target="_blank" rel="noreferrer">在新标签页重新登录</a>
      <Button onClick={() => { void (async () => {
        try {
          const { data } = await bundle.api.raw.GET("/auth/session"); if (!data) throw new Error("会话尚未恢复");
          bundle.queryClient.setQueryData(queryKeys.auth.session(), data);
          writeAccess.resume(); setExpired(false); await active.current?.retrySave();
          await refreshTree();
        } catch (cause) { setError(message(cause)); }
      })(); }}>已重新登录，重试原版本</Button>
    </Alert>}
    {receipts.map(receipt => <Alert key={receipt.id} tone="info" title={`已删除：${receipt.title}`}>
      <Button disabled={working || expired} onClick={() => void undo(receipt)}>恢复本次删除 {receipt.title}</Button>
      {rootRecovery === receipt.id && <><p>原父节点不可用或恢复冲突。可明确恢复到根目录。</p><Button disabled={working || expired} onClick={() => void undo(receipt, true)}>恢复到根 {receipt.title}</Button></>}
    </Alert>)}
    <div className={noteId ? "notes-workspace notes-workspace--detail" : "notes-workspace"}>
      <aside className="notes-directory min-w-0 space-y-3" aria-label="笔记目录">
        {noteId ? <h2 ref={directoryHeading} tabIndex={-1} className="section-title">笔记目录</h2> : <h1 className="page-title">笔记</h1>}
        {directoryContents}
      </aside>
      <div className="min-w-0 space-y-3">
        {noteId && <div ref={directoryTrigger} className="notes-directory-trigger">
          <Drawer open={openDirectoryFor === noteId} onOpenChange={open => setOpenDirectoryFor(open ? noteId : undefined)} title="笔记目录" trigger={<Button size="sm" variant="outline">目录</Button>}>
            <div className="space-y-3">
              {error && <Alert tone="error">{error}</Alert>}
              {directoryContents}
            </div>
          </Drawer>
        </div>}
        {initialNote ? <NotePane key={initialNote.id} initialNote={initialNote} draftStore={draftStore} bundle={bundle} notes={directory ?? []} expired={expired} working={working}
          onCreateChild={() => void create(initialNote.id)} onSession={setActiveSession} assertCanWrite={() => assertCanWrite(initialNote.id)} onBusy={setBusy} onRefresh={refreshTree} onError={setError}
          onDeleted={async receipt => {
            writeAccess.markDeleted(receipt.id); setReceipts(previous => [...previous, receipt]);
            bypass.current = "/notes";
            await navigate({ to: "/notes" });
            active.current = null; draftStore.forget(receipt.id);
            setOpened(previous => { const next = { ...previous }; delete next[receipt.id]; return next; });
            bundle.queryClient.removeQueries({ queryKey: queryKeys.notes.detail(receipt.id) });
            await refreshTree();
          }} /> : noteId ? <>
            {detail.isFetching && <Spinner label="正在读取笔记" />}
            {detail.error && <Alert tone="error">{detail.error instanceof ApiRequestError && detail.error.status === 404 ? "笔记不存在或已删除" : message(detail.error)}<Button onClick={() => void detail.refetch()}>重试笔记</Button></Alert>}
          </> : directory && directory.length > 0 ? <div className="notes-directory-hint"><Empty title="选择一篇笔记" /></div> : null}
      </div>
    </div>
  </div>;
}
interface NotePaneProps {
  initialNote: Note; draftStore: NoteDraftStore; bundle: QueryClientBundle; notes: Note[]; expired: boolean; working: boolean;
  assertCanWrite: () => void; onCreateChild: () => void; onSession: (session: NoteEditingSession | null) => void; onBusy: (busy: boolean) => void; onRefresh: () => Promise<void>; onError: (error: string) => void; onDeleted: (receipt: DeleteReceipt) => Promise<void>;
}
function NotePane({ initialNote, draftStore, bundle, notes, expired, working, onCreateChild, onSession, assertCanWrite, onBusy, onRefresh, onError, onDeleted }: NotePaneProps) {
  const autosave = useNoteAutosave({ initialNote, draftStore });
  const session = draftStore.open(initialNote);
  // Own the mounted session independently of server Query removal or background updates.
  useEffect(() => { onSession(session); return () => onSession(null); }, [session, onSession]);
  const pending = useRef(false);
  const [history, setHistory] = useState(false);
  const historyTrigger = useRef<HTMLButtonElement>(null);
  const titleInput = useRef<HTMLInputElement>(null);
  const menuTrigger = useRef<HTMLButtonElement>(null);
  const [moveOpen, setMoveOpen] = useState(false);
  const [moveError, setMoveError] = useState("");
  const focusMoveError = useCallback((node: HTMLDivElement | null) => { node?.focus(); }, []);
  const [parent, setParent] = useState<string | null>(ROOT);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [conflict, setConflict] = useState(false);
  const conflictTrigger = useRef<HTMLButtonElement>(null);
  const unavailable = expired || working;
  async function command(operation: () => Promise<void>) {
    if (pending.current || expired) throw new Error("操作尚未完成或会话已失效");
    pending.current = true; onBusy(true); onError("");
    try { await operation(); } catch (cause) { onError(message(cause)); throw cause; } finally { pending.current = false; onBusy(false); }
  }
  const blockedParents = descendants(notes, initialNote.id);
  const status = { saved: "已保存", dirty: "未保存", saving: "保存中", error: "保存失败", conflict: "保存冲突" }[autosave.status];
  function closeHistory() {
    setHistory(false);
    historyTrigger.current?.focus();
  }
  function closeMove() {
    setMoveOpen(false);
    requestAnimationFrame(() => menuTrigger.current?.focus());
  }
  return <section aria-label="当前笔记" className="notes-pane space-y-4">
    <h1 className="sr-only">{autosave.draft.title || "未命名笔记"}</h1>
    <Input ref={titleInput} aria-label="笔记标题" className="notes-title-input" value={autosave.draft.title}
      maxLength={500} disabled={working} readOnly={expired}
      onChange={event => { if (!pending.current && !expired) autosave.changeDraft({ title: event.target.value }); }} />
    <p role="status" className="text-sm text-muted">{status} · 当前基础版本 {autosave.baseVersion}{autosave.unresolvedWrite ? " · 写入尚未确认" : ""}</p>
    {autosave.error && <Alert tone="error">{autosave.error.message}</Alert>}
    {autosave.historyError && <Alert tone="error">历史保存失败：{autosave.historyError.message}</Alert>}
    {autosave.serverUpdated && <Alert tone="warning">服务器内容已更新；本地草稿仍保留。</Alert>}
    {(autosave.status === "conflict" || autosave.serverUpdated) && <Button ref={conflictTrigger} size="sm" variant="outline" disabled={unavailable}
      onClick={() => setConflict(true)}>版本与冲突处理</Button>}
    <div className="flex flex-wrap gap-2">
      <Button size="sm" variant="solid" disabled={unavailable} onClick={() => { void command(async () => { await autosave.retrySave(); await autosave.flush(); }).catch(() => {}); }}>保存并记录历史</Button>
      <Button size="sm" variant="outline" ref={historyTrigger} aria-expanded={history}
        aria-controls={history ? "note-history" : undefined} onClick={() => { if (history) closeHistory(); else setHistory(true); }}>历史版本</Button>
      <NoteAssociations noteId={initialNote.id} api={bundle.api} readOnly={unavailable} assertCanWrite={assertCanWrite} />
      <Menu trigger={<Button size="sm" variant="ghost" ref={menuTrigger} aria-label="更多笔记操作">更多</Button>} items={[
        { key: "new-child", label: "新建子笔记", disabled: unavailable, onSelect: onCreateChild },
        { key: "move", label: "移动笔记", disabled: unavailable || notes.length === 0, onSelect: () => { setParent(ROOT); setMoveError(""); setMoveOpen(true); } },
        { key: "copy", label: "复制当前草稿", onSelect: () => { void autosave.copyDraft().catch(cause => onError(message(cause))); } },
        ...((autosave.status === "conflict" || autosave.serverUpdated) ? [] : [
          { key: "conflict", label: "版本与冲突处理", disabled: unavailable, onSelect: () => setConflict(true) },
        ]),
        { type: "divider", key: "before-delete" },
        { key: "delete", label: "删除笔记", danger: true, disabled: unavailable, onSelect: () => setDeleteOpen(true) },
      ]} />
    </div>
    {/* The same editor and selection survive history and drawer interactions. */}
    <div className="notes-document reading-measure" hidden={history} inert={history}>
      <NoteEditor key={`${initialNote.id}:${autosave.resetEpoch}`} document={autosave.draft.document} readOnly={unavailable} onChange={document => { if (!pending.current && !expired) autosave.changeDraft({ document }); }}>
        <SlashMenu /><BlockMenu /><ImageBlockControls client={bundle.api} /><CodeBlockControls />
      </NoteEditor>
    </div>
    {history && <HistoryPanel noteId={initialNote.id} bundle={bundle} disabled={unavailable} onClose={closeHistory} onRestore={async snapshot => {
      await command(async () => {
        await autosave.restoreFromServer(async (noteId, baseVersion) => {
          const { data } = await bundle.api.raw.POST("/notes/{noteId}/snapshots/{snapshotId}/restore", { params: { path: { noteId, snapshotId: snapshot.snapshot_id } }, body: { expected_version: baseVersion } });
          if (!data) throw new Error("恢复响应缺少笔记");
          return data;
        });
        await onRefresh();
      });
      setHistory(false);
      requestAnimationFrame(() => titleInput.current?.focus());
    }} />}
    <Dialog open={moveOpen} onOpenChange={open => { if (!open && !pending.current) closeMove(); }} title="移动笔记" description="笔记将移到所选父节点的末尾。">
      {moveError && <div ref={focusMoveError} tabIndex={-1}><Alert tone="error">{moveError}</Alert></div>}
      <Select label="目标父笔记" value={parent} onValueChange={setParent} disabled={unavailable} options={[{ value: ROOT, label: "根目录" }, ...notes.filter(note => !blockedParents.has(note.id)).map(note => ({ value: note.id, label: note.title }))]} />
      <Button disabled={unavailable || parent === null} onClick={() => { setMoveError(""); void command(async () => {
        const parentId = parent === ROOT ? null : parent;
        const position = notes.filter(note => note.parent_id === parentId && note.id !== initialNote.id).length;
        await autosave.runStructuralMutation(async (noteId, baseVersion) => {
          const { data } = await bundle.api.raw.POST("/notes/{noteId}/move", { params: { path: { noteId } }, body: { expected_version: baseVersion, parent_id: parentId, position } });
          if (!data) throw new Error("移动响应缺少笔记"); return data;
        }); await onRefresh(); closeMove();
      }).catch(cause => setMoveError(message(cause))); }}>确认移动</Button>
    </Dialog>
    <ConfirmDialog open={deleteOpen} onOpenChange={open => { setDeleteOpen(open); if (!open) requestAnimationFrame(() => menuTrigger.current?.focus()); }} title="删除笔记" description={notes.some(note => note.parent_id === initialNote.id) ? "此笔记仍有子页。请先移动或删除子页；服务端会拒绝删除有存活子页的笔记。" : "先保存当前内容与历史，再删除；本页提供本次删除的恢复入口。"} confirmText="确认删除" danger onConfirm={() => command(async () => {
      await autosave.flush(); const expectedVersion = session.getSnapshot().baseVersion;
      assertCanWrite();
      await bundle.api.raw.DELETE("/notes/{noteId}", { params: { path: { noteId: initialNote.id }, query: { expected_version: expectedVersion } } });
      const metadata = notes.find(note => note.id === initialNote.id) ?? initialNote;
      await onDeleted({ id: initialNote.id, title: session.getSnapshot().draft.title, originalParent: metadata.parent_id, expectedVersion: expectedVersion + 1 });
    })} />
    <ConflictDialog open={conflict} onOpenChange={open => {
      setConflict(open);
      if (!open) requestAnimationFrame(() => (conflictTrigger.current ?? menuTrigger.current)?.focus());
    }} draft={autosave.draft} readOnly={expired}
      onLoadServer={() => command(async () => { await autosave.loadServer(); })}
      onSaveCopy={title => command(async () => { await autosave.saveCopy(title); await onRefresh(); })} onCopyDraft={autosave.copyDraft} />
  </section>;
}
