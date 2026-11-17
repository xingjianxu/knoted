import { useEffect, useSyncExternalStore } from "react";
import type { QueryClient } from "@tanstack/react-query";
import { ApiRequestError, type ApiClient, type components } from "@knoted/api-client";
import { queryKeys } from "../../lib/query";

export type Note = components["schemas"]["Note"];
export type NoteDraft = Pick<Note, "title" | "document">;
export type Snapshot = components["schemas"]["Snapshot"];
export type SaveStatus = "saved" | "dirty" | "saving" | "error" | "conflict";
export interface NoteEditingState {
  draft: NoteDraft;
  /** Initial editor input. Only explicit reset changes it. */
  initialDocument: Note["document"];
  resetEpoch: number;
  baseVersion: number;
  dirty: boolean;
  /** A PATCH is in flight or has not been acknowledged; equality with an older draft cannot confirm it. */
  unresolvedWrite: boolean;
  status: SaveStatus;
  error: Error | null;
  historyError: Error | null;
  serverUpdated: boolean;
}
const clone = <T,>(value: T): T => structuredClone(value);
// PostgreSQL JSONB / transport object-key order is not a content edit.
function orderedJson(_key: string, value: unknown): unknown {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? Object.fromEntries(Object.entries(value).sort(([left], [right]) => left.localeCompare(right))) : value;
}
const equal = (a: NoteDraft, b: NoteDraft) => JSON.stringify(a, orderedJson) === JSON.stringify(b, orderedJson);
const asError = (error: unknown) => error instanceof Error ? error : new Error(String(error));
export function conflictCopyTitle(title: string): string {
  const suffix = "（冲突副本）";
  return [...(title.trim() || "未命名笔记")].slice(0, 500 - [...suffix].length).join("") + suffix;
}

/** Owned by one authenticated editing workspace; never put drafts in Query. */
export function createNoteDraftStore(options: { api: ApiClient; queryClient: QueryClient; assertCanWrite?: (noteId: string) => void }) {
  const entries = new Map<string, NoteEditingSession>();
  return {
    /** A confirmed deletion ends this session; a restore must open a fresh CAS base. */
    forget(noteId: string) { entries.delete(noteId); },
    open(note: Note): NoteEditingSession {
      let session = entries.get(note.id);
      if (!session) {
        session = new NoteEditingSession(note, options.api, options.queryClient, options.assertCanWrite);
        entries.set(note.id, session);
      }
      return session;
    },
  };
}
export type NoteDraftStore = ReturnType<typeof createNoteDraftStore>;

export class NoteEditingSession {
  private state: NoteEditingState;
  private confirmed: NoteDraft;
  private revision = 0;
  private lastEdit = 0;
  private listeners = new Set<() => void>();
  private timer: ReturnType<typeof setTimeout> | undefined;
  private flight: Promise<void> | undefined;
  private barrier: Promise<Snapshot> | undefined;
  private loading: Promise<Note> | undefined;
  private resetting = false;
  private structural = false;
  private mutation: Promise<Note> | undefined;
  private restoring: Promise<Note> | undefined;
  private afterCommandFlush: Promise<Snapshot> | undefined;

  readonly noteId: string;
  constructor(note: Note, private api: ApiClient, private queryClient: QueryClient, private assertCanWrite: (noteId: string) => void = () => {}) {
    this.noteId = note.id;
    this.confirmed = clone({ title: note.title, document: note.document });
    this.state = { draft: clone(this.confirmed), initialDocument: clone(note.document), resetEpoch: 0,
      baseVersion: note.content_version, dirty: false, unresolvedWrite: false, status: "saved", error: null, historyError: null, serverUpdated: false };
  }
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    const unsubscribe = this.queryClient.getQueryCache().subscribe(() => {
      this.observeServer(this.queryClient.getQueryData<Note>(queryKeys.notes.detail(this.noteId)));
    });
    this.observeServer(this.queryClient.getQueryData<Note>(queryKeys.notes.detail(this.noteId)));
    this.schedule();
    return () => { unsubscribe(); this.listeners.delete(listener); if (!this.listeners.size) this.clearTimer(); };
  };
  private update(patch: Partial<NoteEditingState>) {
    this.state = { ...this.state, ...patch };
    for (const listener of this.listeners) listener();
  }
  private clearTimer() { clearTimeout(this.timer); this.timer = undefined; }
  private schedule() {
    this.clearTimer();
    if (!this.listeners.size || (!this.state.dirty && !this.state.unresolvedWrite) || this.state.error || this.flight || this.barrier || this.resetting || this.structural) return;
    this.timer = setTimeout(() => { this.timer = undefined; void this.save().catch(() => {}); }, Math.max(0, this.lastEdit + 1000 - Date.now()));
  }
  changeDraft = (patch: Partial<NoteDraft>) => {
    const draft = { ...this.state.draft, ...clone(patch) };
    this.revision++; this.lastEdit = Date.now();
    const dirty = !equal(draft, this.confirmed);
    this.update({ draft, dirty, status: this.state.error ? this.state.status : this.flight ? "saving" : dirty || this.state.unresolvedWrite ? "dirty" : "saved" });
    this.schedule();
  };
  observeServer = (note: Note | undefined) => {
    if (note?.id === this.noteId && note.content_version > this.state.baseVersion && !this.state.serverUpdated) this.update({ serverUpdated: true });
  };
  private async cache(note: Note) {
    await this.queryClient.cancelQueries({ queryKey: queryKeys.notes.detail(note.id), exact: true });
    this.queryClient.setQueryData<Note>(queryKeys.notes.detail(note.id), old => old && old.content_version > note.content_version ? old : note);
    await this.queryClient.invalidateQueries({ queryKey: queryKeys.notes.all(), refetchType: "none" });
  }
  private save(): Promise<void> {
    if (this.flight) return this.flight;
    if (this.state.error) return Promise.reject(this.state.error);
    if (!this.state.dirty && !this.state.unresolvedWrite) return Promise.resolve();
    const body = clone(this.state.draft);
    const epoch = this.state.resetEpoch;
    const expected_version = this.state.baseVersion;
    const revision = this.revision;
    const noteId = this.noteId;
    this.clearTimer(); this.update({ status: "saving" });
    const task = (async () => {
      try {
        this.assertCanWrite(noteId);
        this.update({ unresolvedWrite: true });
        const { data } = await this.api.raw.PATCH("/notes/{noteId}", { params: { path: { noteId } }, body: { expected_version, ...body } });
        if (!data) throw new Error("保存响应缺少笔记");
        if (this.state.resetEpoch !== epoch) return;
        this.confirmed = body;
        const dirty = this.revision !== revision && !equal(this.state.draft, body);
        this.update({ baseVersion: data.content_version, dirty, unresolvedWrite: false, status: dirty ? "dirty" : "saved", error: null });
        await this.cache(data);
      } catch (error) {
        const failure = asError(error);
        if (this.state.resetEpoch === epoch) this.update({ error: failure, status: failure instanceof ApiRequestError && failure.status === 409 ? "conflict" : "error" });
        throw failure;
      }
    })();
    this.flight = task.finally(() => { this.flight = undefined; this.schedule(); });
    return this.flight;
  }
  retrySave = async () => {
    if (this.restoring) await this.restoring;
    if (this.loading) await this.loading;
    if (this.mutation) await this.mutation;
    if (this.state.status === "conflict") throw this.state.error;
    this.update({ error: null, status: this.state.dirty || this.state.unresolvedWrite ? "dirty" : "saved" });
    await this.save();
  };
  /** Saves every pending local edit, then creates history. Normal navigation awaits this while mounted. */
  flush = (): Promise<Snapshot> => {
    const command = this.restoring ?? this.mutation;
    if (command) {
      this.afterCommandFlush ??= command.then(() => this.flush()).finally(() => { this.afterCommandFlush = undefined; });
      return this.afterCommandFlush;
    }
    if (this.barrier) return this.barrier;
    this.clearTimer();
    const epoch = this.state.resetEpoch;
    const task = (async () => {
      try {
        if (this.resetting) throw new Error("正在加载服务器内容");
        if (this.state.error) throw this.state.error;
        while (this.flight || this.state.dirty || this.state.unresolvedWrite) await this.save();
        const version = this.state.baseVersion;
        const revision = this.revision;
        this.assertCanWrite(this.noteId);
        const { data } = await this.api.raw.POST("/notes/{noteId}/snapshots", { params: { path: { noteId: this.noteId } } });
        if (!data) throw new Error("历史响应缺少快照");
        if (data.content_version !== version) {
          this.update({ serverUpdated: true });
          throw new Error("服务器内容已更新，本地版本尚未确认历史快照");
        }
        if (this.state.resetEpoch !== epoch || this.revision !== revision || this.state.dirty) throw new Error("历史保存期间仍有新编辑，请再次保存");
        this.update({ historyError: null });
        await this.queryClient.invalidateQueries({ queryKey: queryKeys.notes.snapshots(this.noteId), refetchType: "none" });
        if (this.state.resetEpoch !== epoch || this.revision !== revision || this.state.dirty) throw new Error("历史保存期间仍有新编辑，请再次保存");
        return data;
      } catch (error) { const failure = asError(error); this.update({ historyError: failure }); throw failure; }
    })();
    this.barrier = task.finally(() => { this.barrier = undefined; this.schedule(); });
    return this.barrier;
  };
  private replace = (getNote: () => Promise<Note>): Promise<Note> => {
    if (this.loading) return this.loading;
    this.resetting = true; this.clearTimer();
    const revision = this.revision;
    const task = (async () => {
      if (this.mutation) await this.mutation.catch(() => {});
      if (this.flight) await this.flight.catch(() => {});
      if (this.barrier) await this.barrier.catch(() => {});
      const note = await getNote();
      if (revision !== this.revision) throw new Error("加载期间仍有新编辑，已保留本地草稿");
      if (note.id !== this.noteId) throw new Error("不能替换其他笔记的编辑会话");
      this.confirmed = clone({ title: note.title, document: note.document });
      this.update({ draft: clone(this.confirmed), initialDocument: clone(note.document), baseVersion: note.content_version,
        resetEpoch: this.state.resetEpoch + 1, dirty: false, unresolvedWrite: false, status: "saved", error: null, historyError: null, serverUpdated: false });
      await this.cache(note);
      return note;
    })();
    this.loading = task.finally(() => { this.loading = undefined; this.resetting = false; this.schedule(); });
    return this.loading;
  };
  /** Serialize our own MOVE after history; only same-content responses may advance CAS without replacing the editor. */
  runStructuralMutation = (command: (noteId: string, baseVersion: number) => Promise<Note>): Promise<Note> => {
    if (this.mutation || this.restoring || this.resetting) return Promise.reject(new Error("结构操作尚未完成"));
    this.structural = true; this.clearTimer();
    const flushed = this.flush();
    const task = (async () => {
      await flushed;
      const expected = clone(this.confirmed);
      const base = this.state.baseVersion;
      try {
        this.assertCanWrite(this.noteId);
        const note = await command(this.noteId, base);
        if (note.id !== this.noteId || note.content_version < base || !equal({ title: note.title, document: note.document }, expected)) {
          this.update({ serverUpdated: true });
          throw new Error("结构操作响应包含不同内容，请明确加载服务器版本");
        }
        this.update({ baseVersion: note.content_version });
        await this.cache(note);
        return note;
      } catch (error) {
        const failure = asError(error);
        this.update({ error: failure, status: failure instanceof ApiRequestError && failure.status === 409 ? "conflict" : "error" });
        throw failure;
      }
    })();
    this.mutation = task.finally(() => { this.mutation = undefined; this.structural = false; this.schedule(); });
    return this.mutation;
  };
  /** T31 history restore owns the whole request, so late editor callbacks cannot race a PATCH or silently disappear. */
  restoreFromServer = (command: (noteId: string, baseVersion: number) => Promise<Note>): Promise<Note> => {
    if (this.restoring || this.mutation || this.resetting) return Promise.reject(new Error("编辑会话操作尚未完成"));
    this.structural = true; this.clearTimer();
    const flushed = this.flush();
    let commandStarted = false;
    let responseReceived = false;
    const task = (async () => {
      try {
        await flushed;
        return await this.replace(async () => {
          this.assertCanWrite(this.noteId);
          commandStarted = true;
          const note = await command(this.noteId, this.state.baseVersion);
          responseReceived = true;
          return note;
        });
      } catch (error) {
        const failure = asError(error);
        if (commandStarted) this.update({ error: failure, serverUpdated: responseReceived || this.state.serverUpdated,
          status: responseReceived || failure instanceof ApiRequestError && failure.status === 409 ? "conflict" : "error" });
        throw failure;
      }
    })();
    this.restoring = task.finally(() => { this.restoring = undefined; this.structural = false; this.schedule(); });
    return this.restoring;
  };
  loadServer = (): Promise<Note> => this.restoring ? Promise.reject(new Error("历史恢复尚未完成")) : this.replace(async () => {
    const { data } = await this.api.raw.GET("/notes/{noteId}", { params: { path: { noteId: this.noteId } }, cache: "no-store" });
    if (!data) throw new Error("加载响应缺少笔记");
    return data;
  });
  /** Install an already confirmed full replacement; history commands use restoreFromServer. */
  resetFromServer = (note: Note): Promise<Note> => this.restoring ? Promise.reject(new Error("历史恢复尚未完成")) : this.replace(async () => clone(note));
  saveCopy = async (title = conflictCopyTitle(this.state.draft.title)): Promise<Note> => {
    if (!title.trim() || [...title].length > 500) throw new Error("副本标题须为 1 至 500 个字符");
    const captured = clone(this.state.draft);
    this.assertCanWrite(this.noteId);
    const { data } = await this.api.raw.POST("/notes", { body: { ...captured, title, parent_id: null } });
    if (!data) throw new Error("副本响应缺少笔记");
    await this.cache(data);
    return data;
  };
  copyDraft = async () => {
    if (!navigator.clipboard?.writeText) throw new Error("浏览器不支持复制草稿");
    await navigator.clipboard.writeText(JSON.stringify(clone(this.state.draft), null, 2));
  };
}

export function useNoteAutosave({ initialNote, draftStore }: { initialNote: Note; draftStore: NoteDraftStore }) {
  const session = draftStore.open(initialNote);
  const state = useSyncExternalStore(session.subscribe, session.getSnapshot, session.getSnapshot);
  useEffect(() => { session.observeServer(initialNote); }, [initialNote, session]);
  useEffect(() => {
    const beforeUnload = (event: BeforeUnloadEvent) => {
      const current = session.getSnapshot();
      if (current.dirty || current.unresolvedWrite || current.error) { event.preventDefault(); event.returnValue = ""; }
    };
    const pagehide = () => { void session.flush().catch(() => {}); };
    window.addEventListener("beforeunload", beforeUnload);
    window.addEventListener("pagehide", pagehide);
    return () => { window.removeEventListener("beforeunload", beforeUnload); window.removeEventListener("pagehide", pagehide); };
  }, [session]);
  return { ...state, changeDraft: session.changeDraft, retrySave: session.retrySave, flush: session.flush,
    runStructuralMutation: session.runStructuralMutation, loadServer: session.loadServer, restoreFromServer: session.restoreFromServer, resetFromServer: session.resetFromServer, saveCopy: session.saveCopy, copyDraft: session.copyDraft };
}
