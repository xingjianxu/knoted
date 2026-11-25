import { useCallback, useContext, useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useBlocker, useNavigate, useRouterState } from "@tanstack/react-router";
import {
  Alert, Button, ConfirmDialog, Dialog, Divider, Drawer, Empty, Form, Input, Link,
  Popover, Sidebar, Spinner, Splitter, Status, Toolbar, Tooltip, Tree, TreeSelect,
} from "@bixlab/mtone";
import {
  Copy, Ellipsis, FilePlus2, FolderInput, GitCompareArrows, History, PanelLeft, Plus, Save, Trash2,
} from "lucide-react";
import { ApiRequestError } from "@knoted/api-client";
import type { QueryClientBundle } from "../../lib/query";
import { queryKeys } from "../../lib/query";
import { createNoteDraftStore, useNoteAutosave, type Note, type NoteDraftStore, type NoteEditingSession, type SaveStatus } from "./autosave";
import { ConflictDialog } from "./ConflictDialog";
import { HistoryPanel } from "./HistoryPanel";
import { NoteEditor } from "./editor";
import { NoteAssociations } from "./links/NoteAssociations";
import { NotesNavigationBlocked } from "./navigation";
import { descendants, noteQuery, notesTreeQuery, noteTree } from "./queries";
import "./notes-workspace.css";

interface DeleteReceipt { id: string; title: string; originalParent: string | null; expectedVersion: number; }
const message = (cause: unknown) => cause instanceof Error ? cause.message : "操作失败，请重试。";
const ROOT = "root";
const statusLabels: Readonly<Record<SaveStatus, string>> = {
  saved: "已保存",
  dirty: "未保存",
  saving: "保存中",
  error: "保存失败",
  conflict: "保存冲突",
};
const statusTones = {
  saved: "green",
  dirty: "orange",
  saving: "blue",
  error: "red",
  conflict: "red",
} as const satisfies Record<SaveStatus, "green" | "orange" | "blue" | "red">;

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
  const registerSession = useCallback((session: NoteEditingSession) => {
    active.current = session;
    return () => { if (active.current === session) active.current = null; };
  }, []);
  const busy = useRef(false);
  const bypass = useRef<string | null>(null);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState("");
  const [expanded, setExpanded] = useState<string[]>([]);
  const [directorySelection, setDirectorySelection] = useState<string>();
  const [pinnedDefaultId, setPinnedDefaultId] = useState<string>();
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

  const tree = useQuery(notesTreeQuery(bundle));
  // Titles can use confirmed detail responses immediately. Parent/position stay
  // owned by the complete directory read; local drafts never enter this view.
  const directory = tree.data?.map(note => {
    const confirmed = bundle.queryClient.getQueryData<Note>(queryKeys.notes.detail(note.id));
    return confirmed && confirmed.content_version >= note.content_version ? { ...note, title: confirmed.title } : note;
  });
  const liveDirectory = directory?.filter(note => !writeAccess.isDeleted(note.id));
  const treeItems = liveDirectory ? noteTree(liveDirectory) : [];
  const firstRootId = treeItems[0]?.id;

  // Keep the first root note open by default on /notes without jumping if that note moves.
  if (noteId && pinnedDefaultId !== undefined) {
    setPinnedDefaultId(undefined);
  } else if (!noteId && pinnedDefaultId && (!liveDirectory?.some(note => note.id === pinnedDefaultId))) {
    setPinnedDefaultId(undefined);
  } else if (!noteId && !pinnedDefaultId && firstRootId) {
    setPinnedDefaultId(firstRootId);
  }
  const activeNoteId = noteId ?? (pinnedDefaultId && !writeAccess.isDeleted(pinnedDefaultId) ? pinnedDefaultId : firstRootId);

  // The opener stays mounted as a completed navigation closes the drawer.
  if (openDirectoryFor && openDirectoryFor !== activeNoteId) setOpenDirectoryFor(undefined);
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

  const detail = useQuery({ ...noteQuery(bundle, activeNoteId ?? ""), enabled: Boolean(activeNoteId) && !writeAccess.isDeleted(activeNoteId!) });

  // Reveal a direct-linked child once its complete directory becomes available.
  if (directory && directorySelection !== activeNoteId) {
    const current = directory.find(note => note.id === activeNoteId);
    if (!activeNoteId || current) {
      setDirectorySelection(activeNoteId);
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
  const initialNote = activeNoteId ? opened[activeNoteId] ?? detail.data : undefined;

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
  function selectNote(id: string) {
    if (id === activeNoteId) {
      setOpenDirectoryFor(undefined);
      return;
    }
    void navigate({ to: "/notes/$noteId", params: { noteId: id } });
  }

  const directoryBody = (
    <div className="space-y-2">
      {tree.isFetching && <Spinner label="正在读取完整笔记树" />}
      {tree.error && (
        <Alert tone="error">
          目录读取失败：{tree.error.message}
          <div className="mt-2">
            <Button size="sm" onClick={() => void tree.refetch()}>重试目录</Button>
          </div>
        </Alert>
      )}
      {treeItems.length > 0 && (
        <Tree
          className="notes-tree"
          label="笔记树"
          items={treeItems}
          directory
          expandAction={false}
          indent={14}
          gap={2}
          selectedKeys={activeNoteId ? [activeNoteId] : []}
          expandedKeys={expanded}
          onExpand={keys => setExpanded(keys.map(String))}
          onSelect={keys => {
            const id = keys[0];
            if (typeof id === "string") selectNote(id);
          }}
        />
      )}
      {liveDirectory?.length === 0 && !tree.isFetching && !tree.error && <Empty title="暂无笔记" />}
    </div>
  );

  const narrowDirectoryDrawer = activeNoteId ? (
    <div ref={directoryTrigger} className="notes-directory-trigger">
      <Drawer
        open={openDirectoryFor === activeNoteId}
        onOpenChange={open => setOpenDirectoryFor(open ? activeNoteId : undefined)}
        title="笔记目录"
        trigger={
          <Button size="sm" variant="outline">
            <PanelLeft size={15} aria-hidden="true" />
            目录
          </Button>
        }
      >
        <div className="space-y-3">
          <div className="flex justify-end">
            <Button size="sm" variant="outline" disabled={working || expired} onClick={() => void create(null)}>
              <Plus size={14} aria-hidden="true" />
              新建根笔记
            </Button>
          </div>
          {error && <Alert tone="error">{error}</Alert>}
          {directoryBody}
        </div>
      </Drawer>
    </div>
  ) : null;

  const hasBanners = Boolean(error || expired || receipts.length > 0);

  return (
    <div className="notes-page">
      {hasBanners && (
        <div className="notes-banners space-y-2">
          {error && <Alert tone="error">{error}</Alert>}
          {expired && (
            <Alert tone="warning" title="会话已失效，本地草稿已保留">
              <p>远程写入已停止。请先复制草稿，或在新标签页重新登录，再明确重试原版本。</p>
              <div className="mt-2 flex flex-wrap items-center gap-2">
                <Link href={`/login?redirect=${encodeURIComponent(pathname)}`} target="_blank">在新标签页重新登录</Link>
                <Button size="sm" variant="outline" onClick={() => { void (async () => {
                  try {
                    const { data } = await bundle.api.raw.GET("/auth/session"); if (!data) throw new Error("会话尚未恢复");
                    bundle.queryClient.setQueryData(queryKeys.auth.session(), data);
                    writeAccess.resume(); setExpired(false); await active.current?.retrySave();
                    await refreshTree();
                  } catch (cause) { setError(message(cause)); }
                })(); }}>已重新登录，重试原版本</Button>
              </div>
            </Alert>
          )}
          {receipts.map(receipt => (
            <Alert key={receipt.id} tone="info" title={`已删除：${receipt.title}`}>
              <div className="mt-2 flex flex-wrap items-center gap-2">
                <Button size="sm" disabled={working || expired} onClick={() => void undo(receipt)}>恢复本次删除 {receipt.title}</Button>
                {rootRecovery === receipt.id && (
                  <>
                    <span className="text-sm text-muted">原父节点不可用或恢复冲突。</span>
                    <Button size="sm" variant="outline" disabled={working || expired} onClick={() => void undo(receipt, true)}>恢复到根 {receipt.title}</Button>
                  </>
                )}
              </div>
            </Alert>
          ))}
        </div>
      )}

      <Splitter
        orientation="horizontal"
        className={activeNoteId ? "notes-splitter notes-splitter--detail" : "notes-splitter"}
      >
        <Splitter.Panel
          id="notes-sidebar"
          label="笔记目录"
          defaultSize={260}
          min="14%"
          max="45%"
          collapsible
          groupResizeBehavior="preserve-pixel-size"
          className="notes-splitter-sidebar"
        >
          <Sidebar
            className="notes-sidebar"
            aria-label="笔记目录"
            header={
              <div className="notes-sidebar-header">
                {activeNoteId ? (
                  <h2 ref={directoryHeading} tabIndex={-1} className="notes-sidebar-title">笔记目录</h2>
                ) : (
                  <h1 className="notes-sidebar-title">笔记</h1>
                )}
                <Tooltip content="新建根笔记" side="bottom">
                  <Button
                    size="sm"
                    variant="ghost"
                    iconOnly
                    disabled={working || expired}
                    aria-label="新建根笔记"
                    onClick={() => void create(null)}
                  >
                    <Plus size={16} aria-hidden="true" />
                  </Button>
                </Tooltip>
              </div>
            }
          >
            {directoryBody}
          </Sidebar>
        </Splitter.Panel>

        <Splitter.Panel
          id="notes-content"
          label="笔记内容"
          min="40%"
          className="notes-splitter-content"
        >
          <div className="notes-content-shell">
            {narrowDirectoryDrawer}
            {initialNote ? (
              <NotePane
                key={initialNote.id}
                initialNote={initialNote}
                draftStore={draftStore}
                bundle={bundle}
                notes={liveDirectory ?? []}
                expired={expired}
                working={working}
                onCreateChild={() => void create(initialNote.id)}
                onSession={registerSession}
                assertCanWrite={() => assertCanWrite(initialNote.id)}
                onBusy={setBusy}
                onRefresh={refreshTree}
                onError={setError}
                onDeleted={async receipt => {
                  writeAccess.markDeleted(receipt.id);
                  setReceipts(previous => [...previous, receipt]);
                  if (pinnedDefaultId === receipt.id) setPinnedDefaultId(undefined);
                  if (active.current?.noteId === receipt.id) active.current = null;
                  draftStore.forget(receipt.id);
                  setOpened(previous => { const next = { ...previous }; delete next[receipt.id]; return next; });
                  bundle.queryClient.removeQueries({ queryKey: queryKeys.notes.detail(receipt.id) });
                  if (pathname !== "/notes") {
                    bypass.current = "/notes";
                    try {
                      await navigate({ to: "/notes" });
                    } finally {
                      bypass.current = null;
                    }
                  }
                  await refreshTree();
                }}
              />
            ) : activeNoteId ? (
              <div className="notes-pane">
                <div className="notes-body-scroll">
                  <div className="notes-body-inner space-y-3">
                    {detail.isFetching && <Spinner label="正在读取笔记" />}
                    {detail.error && (
                      <Alert tone="error">
                        {detail.error instanceof ApiRequestError && detail.error.status === 404 ? "笔记不存在或已删除" : message(detail.error)}
                        <div className="mt-2">
                          <Button size="sm" onClick={() => void detail.refetch()}>重试笔记</Button>
                        </div>
                      </Alert>
                    )}
                  </div>
                </div>
              </div>
            ) : (
              <div className="notes-empty-pane">
                {tree.isFetching ? (
                  <Spinner label="正在读取笔记" />
                ) : liveDirectory?.length === 0 && !tree.error ? (
                  <Empty
                    title="暂无笔记"
                    action={
                      <Button size="sm" variant="solid" disabled={working || expired} onClick={() => void create(null)}>
                        <Plus size={14} aria-hidden="true" />
                        新建笔记
                      </Button>
                    }
                  />
                ) : null}
              </div>
            )}
          </div>
        </Splitter.Panel>
      </Splitter>
    </div>
  );
}

interface NotePaneProps {
  initialNote: Note;
  draftStore: NoteDraftStore;
  bundle: QueryClientBundle;
  notes: Note[];
  expired: boolean;
  working: boolean;
  assertCanWrite: () => void;
  onCreateChild: () => void;
  onSession: (session: NoteEditingSession) => () => void;
  onBusy: (busy: boolean) => void;
  onRefresh: () => Promise<void>;
  onError: (error: string) => void;
  onDeleted: (receipt: DeleteReceipt) => Promise<void>;
}

function NotePane({
  initialNote, draftStore, bundle, notes, expired, working,
  onCreateChild, onSession, assertCanWrite, onBusy, onRefresh, onError, onDeleted,
}: NotePaneProps) {
  const autosave = useNoteAutosave({ initialNote, draftStore });
  const session = draftStore.open(initialNote);
  // Own the mounted session independently of server Query removal or background updates.
  useEffect(() => onSession(session), [session, onSession]);
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
  const status = statusLabels[autosave.status];
  const hasConflictAlert = autosave.status === "conflict" || autosave.serverUpdated;

  function closeHistory() {
    setHistory(false);
    historyTrigger.current?.focus();
  }
  function closeMove() {
    setMoveOpen(false);
    requestAnimationFrame(() => menuTrigger.current?.focus());
  }

  return (
    <section aria-label="当前笔记" className="notes-pane">
      <h1 className="sr-only">{autosave.draft.title || "未命名笔记"}</h1>
      <div className="notes-pane-bar">
        <Toolbar
          background="white"
          className="notes-pane-toolbar"
          aria-label="笔记操作"
          start={
            <p role="status" className="notes-status">
              <Status tone={statusTones[autosave.status]}>{status}</Status>
              {" "}
              <span className="text-muted">· 当前基础版本 {autosave.baseVersion}{autosave.unresolvedWrite ? " · 写入尚未确认" : ""}</span>
            </p>
          }
          end={
            <>
              {hasConflictAlert && (
                <Button ref={conflictTrigger} size="sm" variant="outline" disabled={unavailable} onClick={() => setConflict(true)}>
                  <GitCompareArrows size={14} aria-hidden="true" />
                  版本与冲突处理
                </Button>
              )}
              <Button size="sm" variant="solid" disabled={unavailable} onClick={() => { void command(async () => { await autosave.retrySave(); await autosave.flush(); }).catch(() => {}); }}>
                <Save size={14} aria-hidden="true" />
                保存并记录历史
              </Button>
              <Button
                size="sm"
                variant={history ? "soft" : "outline"}
                ref={historyTrigger}
                aria-expanded={history}
                aria-controls={history ? "note-history" : undefined}
                onClick={() => { if (history) closeHistory(); else setHistory(true); }}
              >
                <History size={14} aria-hidden="true" />
                历史版本
              </Button>
              <NoteAssociations noteId={initialNote.id} api={bundle.api} readOnly={unavailable} assertCanWrite={assertCanWrite} />
              <Popover.Menu
                align="end"
                label="更多笔记操作"
                trigger={
                  <Button size="sm" variant="ghost" iconOnly ref={menuTrigger} aria-label="更多笔记操作" title="更多笔记操作">
                    <Ellipsis size={16} aria-hidden="true" />
                  </Button>
                }
                items={[
                  { key: "new-child", label: "新建子笔记", icon: <FilePlus2 size={15} aria-hidden="true" />, disabled: unavailable, onSelect: onCreateChild },
                  { key: "move", label: "移动笔记", icon: <FolderInput size={15} aria-hidden="true" />, disabled: unavailable || notes.length === 0, onSelect: () => { setParent(ROOT); setMoveError(""); setMoveOpen(true); } },
                  { key: "copy", label: "复制当前草稿", icon: <Copy size={15} aria-hidden="true" />, onSelect: () => { void autosave.copyDraft().catch(cause => onError(message(cause))); } },
                  ...(hasConflictAlert ? [] : [
                    { key: "conflict", label: "版本与冲突处理", icon: <GitCompareArrows size={15} aria-hidden="true" />, disabled: unavailable, onSelect: () => setConflict(true) },
                  ]),
                  { type: "divider", key: "before-delete" },
                  { key: "delete", label: "删除笔记", icon: <Trash2 size={15} aria-hidden="true" />, danger: true, disabled: unavailable, onSelect: () => setDeleteOpen(true) },
                ]}
              />
            </>
          }
        />
      </div>

      <div className="notes-body-scroll">
        <div className="notes-body-inner space-y-4">
          {autosave.error && <Alert tone="error">{autosave.error.message}</Alert>}
          {autosave.historyError && <Alert tone="error">历史保存失败：{autosave.historyError.message}</Alert>}
          {autosave.serverUpdated && <Alert tone="warning">服务器内容已更新；本地草稿仍保留。</Alert>}

          <Input
            ref={titleInput}
            variant="borderless"
            aria-label="笔记标题"
            placeholder="未命名笔记"
            className="notes-title-input"
            value={autosave.draft.title}
            maxLength={500}
            disabled={working}
            readOnly={expired}
            onChange={event => { if (!pending.current && !expired) autosave.changeDraft({ title: event.target.value }); }}
          />
          <Divider />

          {/* The same editor and selection survive history and drawer interactions. */}
          <div className="notes-document reading-measure" hidden={history} inert={history}>
            <NoteEditor
              key={`${initialNote.id}:${autosave.resetEpoch}`}
              documentKey={`${initialNote.id}:${autosave.resetEpoch}`}
              document={autosave.draft.document}
              readOnly={unavailable}
              client={bundle.api}
              onChange={document => { if (!pending.current && !expired) autosave.changeDraft({ document }); }}
            />
          </div>

          {history && (
            <HistoryPanel
              noteId={initialNote.id}
              bundle={bundle}
              disabled={unavailable}
              onClose={closeHistory}
              onRestore={async snapshot => {
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
              }}
            />
          )}
        </div>
      </div>

      <Dialog open={moveOpen} onOpenChange={open => { if (!open && !pending.current) closeMove(); }} title="移动笔记" description="笔记将移到所选父节点的末尾。">
        <Form
          className="space-y-4"
          onSubmit={event => {
            event.preventDefault();
            if (unavailable || parent === null) return;
            setMoveError("");
            void command(async () => {
              const parentId = parent === ROOT ? null : parent;
              const position = notes.filter(note => note.parent_id === parentId && note.id !== initialNote.id).length;
              await autosave.runStructuralMutation(async (noteId, baseVersion) => {
                const { data } = await bundle.api.raw.POST("/notes/{noteId}/move", { params: { path: { noteId } }, body: { expected_version: baseVersion, parent_id: parentId, position } });
                if (!data) throw new Error("移动响应缺少笔记");
                return data;
              });
              await onRefresh();
              closeMove();
            }).catch(cause => setMoveError(message(cause)));
          }}
        >
          {moveError && <div ref={focusMoveError} tabIndex={-1}><Alert tone="error">{moveError}</Alert></div>}
          <Form.Item label="目标父笔记">
            <TreeSelect
              aria-label="目标父笔记"
              value={parent}
              onValueChange={value => setParent(value === null ? null : String(value))}
              disabled={unavailable}
              searchable
              defaultExpandAll
              items={[{ id: ROOT, label: "根目录" }, ...noteTree(notes.filter(note => !blockedParents.has(note.id)))]}
            />
          </Form.Item>
          <div className="flex flex-wrap justify-end gap-2">
            <Button type="button" disabled={unavailable} onClick={closeMove}>取消</Button>
            <Button type="submit" variant="solid" disabled={unavailable || parent === null}>确认移动</Button>
          </div>
        </Form>
      </Dialog>

      <ConfirmDialog
        open={deleteOpen}
        onOpenChange={open => { setDeleteOpen(open); if (!open) requestAnimationFrame(() => menuTrigger.current?.focus()); }}
        title="删除笔记"
        description={notes.some(note => note.parent_id === initialNote.id) ? "此笔记仍有子页。请先移动或删除子页；服务端会拒绝删除有存活子页的笔记。" : "先保存当前内容与历史，再删除；本页提供本次删除的恢复入口。"}
        confirmText="确认删除"
        danger
        onConfirm={() => command(async () => {
          await autosave.flush();
          const expectedVersion = session.getSnapshot().baseVersion;
          assertCanWrite();
          await bundle.api.raw.DELETE("/notes/{noteId}", { params: { path: { noteId: initialNote.id }, query: { expected_version: expectedVersion } } });
          const metadata = notes.find(note => note.id === initialNote.id) ?? initialNote;
          await onDeleted({ id: initialNote.id, title: session.getSnapshot().draft.title, originalParent: metadata.parent_id, expectedVersion: expectedVersion + 1 });
        })}
      />

      <ConflictDialog
        open={conflict}
        onOpenChange={open => {
          setConflict(open);
          if (!open) requestAnimationFrame(() => (conflictTrigger.current ?? menuTrigger.current)?.focus());
        }}
        draft={autosave.draft}
        readOnly={expired}
        onLoadServer={() => command(async () => { await autosave.loadServer(); })}
        onSaveCopy={title => command(async () => { await autosave.saveCopy(title); await onRefresh(); })}
        onCopyDraft={autosave.copyDraft}
      />
    </section>
  );
}
