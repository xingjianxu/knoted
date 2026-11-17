import { act, renderHook, render, fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, afterAll, expect, it, vi } from "vitest";
import { QueryClient } from "@tanstack/react-query";
import { createApiClient, type components } from "@knoted/api-client";
import { createNoteDraftStore, useNoteAutosave } from "../../src/features/notes/autosave";

import { useEffect, useState } from "react";
import { useBlockNoteEditor } from "@blocknote/react";
import { NoteEditor, noteEditorSchema, type NoteEditorInstance } from "../../src/features/notes/editor";
import { ConflictDialog } from "../../src/features/notes/ConflictDialog";
import { UiProvider } from "../../src/lib/ui-provider";

type Note = components["schemas"]["Note"];
const doc = (text = "initial") => ({ blocks: [{ id: "p", type: "paragraph" as const, content: [{ type: "text" as const, text }] }] });
const note = (id = "a", version = 1, title = "initial"): Note => ({ id, title, document: doc(title), content_version: version, parent_id: null, position: 0, format_version: 1, created_at: "2026-11-17T00:00:00Z", updated_at: "2026-11-17T00:00:00Z", deleted_at: null });
const json = (data: unknown, status = 200) => new Response(JSON.stringify(data), { status, headers: { "Content-Type": "application/json" } });
function deferred() { let resolve!: (r: Response) => void; let reject!: (e: Error) => void; const promise = new Promise<Response>((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; }
function setup(fetcher: typeof fetch) {
  const requests: Request[] = [];
  const api = createApiClient({ baseUrl: "http://localhost:3000/api/v1", fetch: (input, init) => { requests.push(input as Request); return fetcher(input, init); } });
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: Infinity } } });
  const store = createNoteDraftStore({ api, queryClient });
  const view = renderHook(({ initialNote }) => useNoteAutosave({ initialNote, draftStore: store }), { initialProps: { initialNote: note() } });
  return { ...view, store, requests, queryClient };
}
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); document.cookie = "knoted_csrf=; Max-Age=0; path=/"; });
async function tick(ms = 1000) { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); }
it("t30_debounce_latest_draft uses immutable latest DTO, partial patches, CSRF and no history", async () => {
  vi.useFakeTimers();
  const { result, requests } = setup(async () => json(note("a", 2, "latest")));
  document.cookie = "knoted_csrf=public-t30-fixture; path=/";
  act(() => result.current.changeDraft({ title: "latest" })); await tick(500);
  act(() => result.current.changeDraft({ document: doc("latest doc") })); await tick(999);
  expect(requests).toHaveLength(0); await tick(1);
  expect(requests).toHaveLength(1); expect(requests[0]!.method).toBe("PATCH");
  expect(requests[0]!.headers.get("X-CSRF-Token")).toBe("public-t30-fixture");
  expect(requests[0]!.credentials).toBe("same-origin");
  expect(await requests[0]!.json()).toEqual({ expected_version: 1, title: "latest", document: doc("latest doc") });
  expect(result.current.status).toBe("saved"); expect(result.current.baseVersion).toBe(2);
});
it("t30_1 serializes pending edits and never cleans latest draft on old acknowledgement", async () => {
  vi.useFakeTimers(); const first = deferred(); const second = deferred();
  const { result, requests } = setup(() => requests.length === 1 ? first.promise : second.promise);
  act(() => result.current.changeDraft({ title: "one" })); await tick();
  act(() => result.current.changeDraft({ title: "two" })); await tick(); expect(requests).toHaveLength(1);
  await act(async () => first.resolve(json(note("a", 2, "one")))); await tick(0);
  expect(result.current.draft.title).toBe("two"); expect(result.current.dirty).toBe(true); expect(requests).toHaveLength(2);
  expect(await requests[1]!.json()).toMatchObject({ expected_version: 2, title: "two" });
  await act(async () => second.resolve(json(note("a", 3, "two")))); expect(result.current.status).toBe("saved");
});

const snapshot = (version = 2) => ({ snapshot_id: "snap", note_id: "a", content_version: version, title: "saved", document: doc("saved"), reason: "flush", created_at: "2026-11-17T00:00:00Z" });
const failure = (status: number) => json({ code: "FIXTURE_ERROR", message: "fixture failure", current_version: 9 }, status);
it("t30_query_out_of_order_dirty_and_clean never advances editing base from Query", async () => {
  vi.useFakeTimers(); const { result, queryClient, rerender, requests } = setup(async () => failure(409));
  act(() => queryClient.setQueryData(["notes", "detail", "a"], note("a", 9, "remote")));
  expect(result.current.serverUpdated).toBe(true); expect(result.current.baseVersion).toBe(1); expect(result.current.draft.title).toBe("initial");
  rerender({ initialNote: note("a", 9, "remote") });
  act(() => result.current.changeDraft({ title: "local" }));
  act(() => queryClient.setQueryData(["notes", "detail", "a"], note("a", 8, "older")));
  await tick(); expect(await requests[0]!.json()).toMatchObject({ expected_version: 1, title: "local" });
  expect(result.current.status).toBe("conflict"); expect(result.current.baseVersion).toBe(1); expect(result.current.draft.title).toBe("local");
  expect(result.current.error).toMatchObject({ status: 409, currentVersion: 9 });
  await tick(10000); expect(requests).toHaveLength(1);
});
it("t30_2 network error retains draft and retries original CAS after lost acknowledgement", async () => {
  vi.useFakeTimers(); const { result, requests } = setup(async () => { if (requests.length === 1) throw new Error("offline"); return failure(409); });
  act(() => result.current.changeDraft({ title: "mine" })); await tick();
  expect(result.current.status).toBe("error"); expect(result.current.dirty).toBe(true); expect(result.current.baseVersion).toBe(1);
  await tick(10000); expect(requests).toHaveLength(1);
  await act(async () => { await expect(result.current.retrySave()).rejects.toMatchObject({ status: 409 }); });
  expect(await requests[1]!.json()).toMatchObject({ expected_version: 1, title: "mine" });
  expect(result.current.status).toBe("conflict");
  act(() => result.current.changeDraft({ title: "still local" })); await tick(10000);
  expect(requests).toHaveLength(2); expect(result.current.draft.title).toBe("still local");
});
it.each([401, 403, 404, 413, 422, 503])("HTTP %i keeps local draft and pauses autosaving", async status => {
  vi.useFakeTimers(); const { result, requests } = setup(async () => failure(status));
  act(() => result.current.changeDraft({ title: "mine" })); await tick(); await tick(10000);
  expect(requests).toHaveLength(1); expect(result.current.error).toMatchObject({ status });
  expect(result.current.dirty).toBe(true); expect(result.current.baseVersion).toBe(1);
});
it("t30_3 A→B→A retains sessions and one flight per note, late results never mutate B", async () => {
  vi.useFakeTimers(); const pending = deferred(); const { result, rerender, requests } = setup(() => pending.promise);
  act(() => result.current.changeDraft({ title: "A edit" })); await tick();
  rerender({ initialNote: note("b", 7, "B initial") });
  expect(result.current.draft.title).toBe("B initial"); expect(result.current.baseVersion).toBe(7);
  rerender({ initialNote: note("a") });
  expect(result.current.draft.title).toBe("A edit"); act(() => result.current.changeDraft({ title: "A latest" }));
  await tick(); expect(requests).toHaveLength(1);
  rerender({ initialNote: note("b", 7, "B initial") });
  await act(async () => pending.resolve(json(note("a", 2, "A edit"))));
  expect(result.current.draft.title).toBe("B initial"); expect(result.current.baseVersion).toBe(7);
  expect(requests).toHaveLength(1);
  rerender({ initialNote: note("a") }); expect(result.current.draft.title).toBe("A latest"); expect(result.current.baseVersion).toBe(2);
});
it("t30_flush_save_then_snapshot_barrier waits serially, shares Promise and posts no body", async () => {
  const save = deferred(); const history = deferred(); const { result, requests } = setup(() => requests.length === 1 ? save.promise : history.promise);
  act(() => result.current.changeDraft({ title: "saved" }));
  let first!: Promise<unknown>; let second!: Promise<unknown>;
  act(() => { first = result.current.flush(); second = result.current.flush(); });
  expect(first).toBe(second); await act(async () => {}); expect(requests).toHaveLength(1);
  await act(async () => save.resolve(json(note("a", 2, "saved"))));
  expect(requests).toHaveLength(2); expect(requests[1]!.method).toBe("POST"); expect(await requests[1]!.text()).toBe("");
  await act(async () => { history.resolve(json(snapshot(), 201)); await expect(first).resolves.toMatchObject({ content_version: 2 }); });
  expect(result.current.dirty).toBe(false);
});
it("flush rejects conflict without posting history", async () => {
  const { result, requests } = setup(async () => failure(409)); act(() => result.current.changeDraft({ title: "mine" }));
  await act(async () => { await expect(result.current.flush()).rejects.toMatchObject({ status: 409 }); });
  expect(requests).toHaveLength(1); expect(result.current.status).toBe("conflict");
});
it("t30_history_failure retries snapshot only, never resends confirmed PATCH", async () => {
  const { result, requests } = setup(async input => (input as Request).method === "PATCH" ? json(note("a", 2, "saved")) : requests.length === 2 ? failure(503) : json(snapshot(), 201));
  act(() => result.current.changeDraft({ title: "saved" }));
  await act(async () => { await expect(result.current.flush()).rejects.toMatchObject({ status: 503 }); });
  expect(result.current.status).toBe("saved"); expect(result.current.historyError).toBeTruthy();
  await act(async () => { await result.current.flush(); });
  expect(requests.map(r => r.method)).toEqual(["PATCH", "POST", "POST"]); expect(result.current.historyError).toBeNull();
});
it("snapshot mismatch retains original base and cannot claim own history persisted", async () => {
  const { result } = setup(async () => json(snapshot(9), 201));
  await act(async () => { await expect(result.current.flush()).rejects.toThrow("服务器内容已更新"); });
  expect(result.current.baseVersion).toBe(1); expect(result.current.serverUpdated).toBe(true); expect(result.current.historyError).toBeTruthy();
});
it("edits during history flush cannot report successful dirty navigation", async () => {
  const history = deferred(); const { result } = setup(() => history.promise);
  let flush!: Promise<unknown>; act(() => { flush = result.current.flush(); });
  act(() => result.current.changeDraft({ title: "new during history" }));
  await act(async () => { history.resolve(json(snapshot(1), 201)); await expect(flush).rejects.toThrow("仍有新编辑"); });
  expect(result.current.dirty).toBe(true); expect(result.current.draft.title).toBe("new during history");
});
it("t30_load_server waits for flight and atomically replaces document/base/reset epoch", async () => {
  vi.useFakeTimers(); const save = deferred(); const get = deferred(); const { result, requests } = setup(() => requests.length === 1 ? save.promise : get.promise);
  act(() => result.current.changeDraft({ title: "local" })); await tick();
  let load!: Promise<unknown>; act(() => { load = result.current.loadServer(); }); expect(requests).toHaveLength(1);
  await act(async () => save.resolve(json(note("a", 2, "local")))); expect(requests).toHaveLength(2);
  expect(result.current.baseVersion).toBe(2); expect(result.current.resetEpoch).toBe(0);
  await act(async () => { get.resolve(json(note("a", 9, "remote"))); await load; });
  expect(result.current.baseVersion).toBe(9); expect(result.current.draft.title).toBe("remote"); expect(result.current.initialDocument).toEqual(doc("remote")); expect(result.current.resetEpoch).toBe(1);
  await tick(10000); expect(requests).toHaveLength(2);
});
it.each(["wait", "get"])("loadServer rejects stale GET when edits occur during %s", async when => {
  vi.useFakeTimers(); const save = deferred(); const get = deferred(); const { result, requests } = setup(() => requests.length === 1 ? save.promise : get.promise);
  act(() => result.current.changeDraft({ title: "local" })); await tick();
  let load!: Promise<unknown>; act(() => { load = result.current.loadServer(); });
  if (when === "wait") act(() => result.current.changeDraft({ title: "latest" }));
  await act(async () => save.resolve(json(note("a", 2, "local"))));
  if (when === "get") act(() => result.current.changeDraft({ title: "latest" }));
  await act(async () => { get.resolve(json(note("a", 9, "remote"))); await expect(load).rejects.toThrow("仍有新编辑"); });
  expect(result.current.draft.title).toBe("latest"); expect(result.current.baseVersion).toBe(2); expect(result.current.resetEpoch).toBe(0);
});
it("load failure retains draft and conflict, successful reset uses new CAS", async () => {
  vi.useFakeTimers(); const { result, requests } = setup(async () => requests.length === 1 ? failure(409) : requests.length === 2 ? failure(503) : json(note("a", 10, "new")));
  act(() => result.current.changeDraft({ title: "local" })); await tick();
  await act(async () => { await expect(result.current.loadServer()).rejects.toMatchObject({ status: 503 }); });
  expect(result.current.status).toBe("conflict"); expect(result.current.draft.title).toBe("local");
  await act(async () => { await result.current.resetFromServer(note("a", 9, "restored")); });
  act(() => result.current.changeDraft({ title: "new" })); await tick();
  expect(await requests[2]!.json()).toMatchObject({ expected_version: 9 }); expect(result.current.resetEpoch).toBe(1);
});
it("t30_copy_title_root captures LOCAL draft and leaves edits made during copy intact", async () => {
  const pending = deferred(); const { result, requests, queryClient } = setup(() => pending.promise);
  act(() => result.current.changeDraft({ title: "😀".repeat(500), document: doc("local") }));
  let copy!: Promise<unknown>; act(() => { copy = result.current.saveCopy(); }); await act(async () => {});
  const body = await requests[0]!.json(); expect([...body.title]).toHaveLength(500); expect(body.title.endsWith("（冲突副本）")).toBe(true);
  expect(body).toMatchObject({ parent_id: null, document: doc("local") }); expect(requests[0]!.method).toBe("POST");
  act(() => result.current.changeDraft({ title: "newer" }));
  await act(async () => { pending.resolve(json(note("copy", 1, body.title), 201)); await copy; });
  expect(result.current.draft.title).toBe("newer"); expect(result.current.baseVersion).toBe(1); expect(result.current.dirty).toBe(true);
  expect(queryClient.getQueryData(["notes", "detail", "copy"])).toMatchObject({ id: "copy" }); expect(queryClient.getQueryData(["notes", "detail", "a"])).toBeUndefined();
});
it("copy/create/clipboard failures keep local draft and no automatic conflict retry", async () => {
  vi.useFakeTimers(); const { result, requests } = setup(async () => failure(requests.length === 1 ? 409 : 503));
  act(() => result.current.changeDraft({ title: "local" })); await tick();
  await act(async () => { await expect(result.current.saveCopy()).rejects.toMatchObject({ status: 503 }); });
  const clipboard = { writeText: vi.fn().mockRejectedValue(new Error("clipboard denied")) }; vi.stubGlobal("navigator", { clipboard });
  await expect(result.current.copyDraft()).rejects.toThrow("clipboard denied");
  expect(JSON.parse(clipboard.writeText.mock.calls[0]![0])).toEqual({ title: "local", document: doc() });
  expect(result.current.draft.title).toBe("local"); expect(result.current.status).toBe("conflict"); await tick(10000); expect(requests).toHaveLength(2);
});
it("normal navigation awaits flush, failure stays mounted; unmount clears debounce/lifecycle listeners", async () => {
  vi.useFakeTimers(); const pending = deferred(); const { result, requests, unmount } = setup(() => pending.promise);
  const navigate = vi.fn(); act(() => result.current.changeDraft({ title: "mine" }));
  let leaving!: Promise<unknown>; act(() => { leaving = result.current.flush().then(navigate); }); await act(async () => {}); expect(navigate).not.toHaveBeenCalled();
  await act(async () => { pending.resolve(failure(503)); await expect(leaving).rejects.toMatchObject({ status: 503 }); });
  expect(navigate).not.toHaveBeenCalled(); expect(result.current.dirty).toBe(true); unmount();
  window.dispatchEvent(new Event("pagehide")); await tick(10000); expect(requests).toHaveLength(1);
});
it("pagehide attempts explicit chain and beforeunload warns synchronously without promising delivery", async () => {
  const { result, requests } = setup(async input => (input as Request).method === "PATCH" ? json(note("a", 2, "mine")) : json(snapshot(), 201));
  act(() => result.current.changeDraft({ title: "mine" }));
  const warning = new Event("beforeunload", { cancelable: true }); window.dispatchEvent(warning); expect(warning.defaultPrevented).toBe(true);
  await act(async () => { window.dispatchEvent(new Event("pagehide")); });
  expect(requests.map(r => r.method)).toEqual(["PATCH", "POST"]);
});

it("confirmed Query cache never regresses a newer server version when save settles", async () => {
  vi.useFakeTimers(); const pending = deferred(); const { result, queryClient } = setup(() => pending.promise);
  act(() => result.current.changeDraft({ title: "mine" })); await tick();
  act(() => queryClient.setQueryData(["notes", "detail", "a"], note("a", 9, "remote")));
  await act(async () => pending.resolve(json(note("a", 2, "mine"))));
  expect(queryClient.getQueryData(["notes", "detail", "a"])).toMatchObject({ content_version: 9 });
  expect(result.current.baseVersion).toBe(2); expect(result.current.draft.title).toBe("mine"); expect(result.current.serverUpdated).toBe(true);
});
it("edits during flush PATCH drain latest draft before history", async () => {
  const pending = deferred(); const { result, requests } = setup(async input => (input as Request).method === "POST" ? json(snapshot(3), 201) : requests.length === 1 ? pending.promise : json(note("a", 3, "newest")));
  act(() => result.current.changeDraft({ title: "first" }));
  let flush!: Promise<unknown>; act(() => { flush = result.current.flush(); });
  act(() => result.current.changeDraft({ title: "newest" }));
  await act(async () => { pending.resolve(json(note("a", 2, "first"))); await flush; });
  expect(requests.map(r => r.method)).toEqual(["PATCH", "PATCH", "POST"]);
  expect(await requests[1]!.json()).toMatchObject({ expected_version: 2, title: "newest" }); expect(result.current.dirty).toBe(false);
});
it("in-flight failure on A cannot poison B, and returning to A retains failed latest draft", async () => {
  vi.useFakeTimers(); const pending = deferred(); const { result, rerender } = setup(() => pending.promise);
  act(() => result.current.changeDraft({ title: "A local" })); await tick();
  rerender({ initialNote: note("b", 7, "B") }); await act(async () => pending.reject(new Error("offline A")));
  expect(result.current.status).toBe("saved"); expect(result.current.draft.title).toBe("B");
  rerender({ initialNote: note() }); expect(result.current.status).toBe("error"); expect(result.current.draft.title).toBe("A local");
});
it("reset waits for failed old write, old response cannot mutate the newly installed epoch", async () => {
  vi.useFakeTimers(); const pending = deferred(); const { result, requests } = setup(() => pending.promise);
  act(() => result.current.changeDraft({ title: "old edit" })); await tick();
  let reset!: Promise<unknown>; act(() => { reset = result.current.resetFromServer(note("a", 9, "restored")); });
  expect(result.current.resetEpoch).toBe(0);
  await act(async () => { pending.resolve(failure(409)); await reset; });
  expect(result.current.resetEpoch).toBe(1); expect(result.current.baseVersion).toBe(9); expect(result.current.status).toBe("saved");
  expect(result.current.draft.title).toBe("restored"); await tick(10000); expect(requests).toHaveLength(1);
});
beforeAll(() => {
  vi.spyOn(window, "scrollBy").mockImplementation(() => {});
  Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
  Object.defineProperty(Range.prototype, "getBoundingClientRect", { configurable: true, value: () => new DOMRect() });
});
afterAll(() => { vi.restoreAllMocks(); Reflect.deleteProperty(Range.prototype, "getClientRects"); Reflect.deleteProperty(Range.prototype, "getBoundingClientRect"); });
it("t30_editor_instance_and_reset_contract preserves real editor/selection/composition across own ack and clean Query updates", async () => {
  const save = deferred();
  const api = createApiClient({ baseUrl: "http://localhost:3000/api/v1", fetch: async input => {
    const request = input as Request;
    return request.method === "PATCH" ? save.promise : request.method === "GET" ? json(note("a", 9, "remote")) : json(snapshot(2), 201);
  } });
  const queryClient = new QueryClient({ defaultOptions: { queries: { gcTime: Infinity } } });
  const store = createNoteDraftStore({ api, queryClient }); const instances: NoteEditorInstance[] = [];
  let current!: ReturnType<typeof useNoteAutosave>;
  function Probe() { const editor = useBlockNoteEditor(noteEditorSchema); useEffect(() => { instances.push(editor); }, [editor]); return null; }
  function Harness({ initialNote }: { initialNote: Note }) {
    const session = useNoteAutosave({ initialNote, draftStore: store });
    useEffect(() => { current = session; });
    return <UiProvider><NoteEditor key={`${initialNote.id}:${session.resetEpoch}`} document={session.initialDocument} onChange={document => session.changeDraft({ document })}><Probe /></NoteEditor></UiProvider>;
  }
  const view = render(<Harness initialNote={note()} />); await waitFor(() => expect(instances).toHaveLength(1));
  const editor = instances[0]!; act(() => editor.setTextCursorPosition("p", "end"));
  const editable = view.container.querySelector("[contenteditable=true]")!;
  fireEvent.compositionStart(editable);
  act(() => current.changeDraft({ title: "saved title" }));
  let flush!: Promise<unknown>; act(() => { flush = current.flush(); });
  await act(async () => { save.resolve(json(note("a", 2, "saved title"))); await flush; });
  expect(instances).toHaveLength(1); expect(editor.getTextCursorPosition().block.id).toBe("p");
  await act(async () => { await current.runStructuralMutation(async (id, base) => {
    expect(id).toBe("a"); expect(base).toBe(2);
    return { ...note("a", 3, "saved title"), document: doc(), parent_id: "folder" };
  }); });
  expect(instances).toHaveLength(1); expect(current.resetEpoch).toBe(0);
  view.rerender(<Harness initialNote={note("a", 8, "query remote")} />);
  expect(current.baseVersion).toBe(3); expect(current.serverUpdated).toBe(true); expect(instances).toHaveLength(1); expect(editor.getBlock("p")!.content).toMatchObject([{ text: "initial" }]);
  fireEvent.compositionEnd(editable);
  await act(async () => { await current.loadServer(); });
  await waitFor(() => expect(instances).toHaveLength(2)); expect(instances[1]).not.toBe(editor);
  expect(current.baseVersion).toBe(9); expect(current.resetEpoch).toBe(1); expect(instances[1]!.getBlock("p")!.content).toMatchObject([{ text: "remote" }]);
});
it("t30_clipboard_and_dialog uses public mtone controls, blocks duplicate actions and dismisses without clearing conflict", async () => {
  vi.useFakeTimers(); const { result } = setup(async () => failure(409));
  act(() => result.current.changeDraft({ title: "local conflict" })); await tick(); vi.useRealTimers();
  const pending = deferred(); const saveCopy = vi.fn(async () => { await pending.promise; throw new Error("copy fixture failure"); });
  const copyDraft = vi.fn(async () => { throw new Error("clipboard denied"); });
  function DialogHarness() {
    const [open, setOpen] = useState(true);
    return <UiProvider><button onClick={() => setOpen(true)}>重新打开冲突</button><ConflictDialog open={open} draft={result.current.draft} onOpenChange={setOpen} onLoadServer={result.current.loadServer} onSaveCopy={saveCopy} onCopyDraft={copyDraft} /></UiProvider>;
  }
  render(<DialogHarness />); const dialog = await screen.findByRole("dialog", { name: "笔记保存冲突" });
  fireEvent.change(screen.getByRole("textbox", { name: "副本标题" }), { target: { value: "chosen title" } });
  fireEvent.click(screen.getByRole("button", { name: "另存副本" }));
  expect(screen.getByRole("button", { name: "复制草稿" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "另存副本" })); expect(saveCopy).toHaveBeenCalledTimes(1); expect(saveCopy).toHaveBeenCalledWith("chosen title");
  await act(async () => pending.resolve(json({}))); await screen.findByText("copy fixture failure");
  fireEvent.click(screen.getByRole("button", { name: "复制草稿" })); await screen.findByText("clipboard denied");
  fireEvent.keyDown(dialog, { key: "Escape", code: "Escape" });
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(result.current.status).toBe("conflict"); expect(result.current.draft.title).toBe("local conflict");
  fireEvent.click(screen.getByRole("button", { name: "重新打开冲突" })); await screen.findByRole("dialog");
  expect(result.current.dirty).toBe(true);
});
it("own structural MOVE barrier blocks concurrent PATCH, retains pending edits and advances only its own base", async () => {
  vi.useFakeTimers(); const move = deferred(); const { result, requests } = setup(async input => (input as Request).method === "POST" ? json(snapshot(1), 201) : json(note("a", 3, "later")));
  const command = vi.fn(async (id: string, base: number) => { expect(id).toBe("a"); expect(base).toBe(1); return await (await move.promise).json() as Note; });
  let moving!: Promise<Note>; act(() => { moving = result.current.runStructuralMutation(command); }); await tick(0);
  expect(command).toHaveBeenCalledTimes(1);
  act(() => result.current.changeDraft({ title: "later", document: doc("later doc") })); await tick(2000);
  expect(requests.map(r => r.method)).toEqual(["POST"]); expect(result.current.baseVersion).toBe(1);
  await act(async () => { move.resolve(json({ ...note("a", 2), parent_id: "folder" })); await moving; }); await tick(0);
  expect(result.current.resetEpoch).toBe(0); expect(result.current.draft).toEqual({ title: "later", document: doc("later doc") });
  expect(await requests[1]!.json()).toMatchObject({ expected_version: 2, title: "later", document: doc("later doc") });
});
it.each(["failure", "content"])("structural MOVE %s preserves original CAS/draft and pauses autosave", async kind => {
  vi.useFakeTimers(); const moving = deferred(); const { result, requests } = setup(async () => json(snapshot(1), 201));
  let operation!: Promise<unknown>;
  act(() => { operation = result.current.runStructuralMutation(async () => {
    await moving.promise;
    if (kind === "failure") throw new Error("MOVE connection lost");
    return note("a", 9, "remote different content");
  }); });
  await tick(0); act(() => result.current.changeDraft({ title: "pending local" }));
  await act(async () => { moving.resolve(json({})); await expect(operation).rejects.toThrow(); });
  expect(result.current.baseVersion).toBe(1); expect(result.current.draft.title).toBe("pending local"); expect(result.current.dirty).toBe(true);
  if (kind === "content") expect(result.current.serverUpdated).toBe(true);
  await tick(10000); expect(requests).toHaveLength(1);
});
it("concurrent structural commands reject busy rather than acknowledging an unexecuted MOVE", async () => {
  const move = deferred(); const { result } = setup(async () => json(snapshot(1), 201));
  let first!: Promise<unknown>; act(() => { first = result.current.runStructuralMutation(async () => { await move.promise; return note("a", 2); }); });
  const other = vi.fn(async () => note("a", 3));
  const second = result.current.runStructuralMutation(other);
  expect(second).not.toBe(first);
  await act(async () => { await expect(second).rejects.toThrow("结构操作尚未完成"); });
  expect(other).not.toHaveBeenCalled();
  await act(async () => { move.resolve(json({})); await first; });
});
it("load started on A never replaces B while GET settles, and partial patches do not share input objects", async () => {
  const get = deferred(); const { result, rerender } = setup(() => get.promise);
  let load!: Promise<unknown>; act(() => { load = result.current.loadServer(); });
  rerender({ initialNote: note("b", 7, "B") }); const document = doc("B local");
  act(() => result.current.changeDraft({ document })); document.blocks[0]!.content[0]!.text = "caller mutated";
  await act(async () => { get.resolve(json(note("a", 9, "A remote"))); await load; });
  expect(result.current.draft).toEqual({ title: "B", document: doc("B local") }); expect(result.current.baseVersion).toBe(7); expect(result.current.resetEpoch).toBe(0);
});
it("copy validates user titles and clipboard success returns recoverable LOCAL JSON without clearing draft", async () => {
  const { result, requests } = setup(async () => json(note("copy"), 201));
  act(() => result.current.changeDraft({ title: "", document: doc("local clipboard") }));
  await expect(result.current.saveCopy("😀".repeat(501))).rejects.toThrow("500"); expect(requests).toHaveLength(0);
  await act(async () => { await result.current.saveCopy(); }); expect(await requests[0]!.json()).toMatchObject({ title: "未命名笔记（冲突副本）", parent_id: null });
  const writeText = vi.fn< (text: string) => Promise<void> >().mockResolvedValue(undefined); vi.stubGlobal("navigator", { clipboard: { writeText } });
  await result.current.copyDraft(); expect(JSON.parse(writeText.mock.calls[0]![0])).toEqual({ title: "", document: doc("local clipboard") }); expect(result.current.dirty).toBe(true);
});
it("flush rechecks edits triggered during final cache invalidation before allowing navigation", async () => {
  const { result, queryClient } = setup(async () => json(snapshot(1), 201));
  const invalidate = queryClient.invalidateQueries.bind(queryClient);
  vi.spyOn(queryClient, "invalidateQueries").mockImplementation(filters => {
    if (filters?.queryKey?.at(-1) === "snapshots") result.current.changeDraft({ title: "late cache subscriber edit" });
    return invalidate(filters);
  });
  await act(async () => { await expect(result.current.flush()).rejects.toThrow("仍有新编辑"); });
  expect(result.current.dirty).toBe(true);
});
it("explicit retry cannot send PATCH while a server load GET remains in progress", async () => {
  const get = deferred(); const { result, requests } = setup(async input => (input as Request).method === "GET" ? get.promise : json(note("a", 2, "local")));
  let loading!: Promise<unknown>; act(() => { loading = result.current.loadServer(); });
  act(() => result.current.changeDraft({ title: "local" }));
  let retry!: Promise<unknown>; act(() => { retry = result.current.retrySave(); }); await act(async () => {});
  expect(requests.map(r => r.method)).toEqual(["GET"]);
  await act(async () => { get.resolve(json(note("a", 9, "remote"))); await expect(loading).rejects.toThrow("仍有新编辑"); await expect(retry).rejects.toThrow("仍有新编辑"); });
  expect(result.current.draft.title).toBe("local"); expect(result.current.baseVersion).toBe(1);
});
it("restore barrier waits old PATCH/history then installs the returned note and new epoch atomically", async () => {
  vi.useFakeTimers(); const save = deferred(); const restore = deferred(); const { result, requests } = setup(async input => (input as Request).method === "PATCH" ? save.promise : json(snapshot(2), 201));
  act(() => result.current.changeDraft({ title: "saved" })); await tick();
  const command = vi.fn(async (id: string, base: number) => { expect(id).toBe("a"); expect(base).toBe(2); return await (await restore.promise).json() as Note; });
  let restoring!: Promise<Note>; act(() => { restoring = result.current.restoreFromServer(command); });
  await tick(0); expect(command).not.toHaveBeenCalled(); expect(requests).toHaveLength(1);
  await act(async () => save.resolve(json(note("a", 2, "saved")))); expect(command).toHaveBeenCalledTimes(1);
  expect(result.current.resetEpoch).toBe(0); expect(requests.map(r => r.method)).toEqual(["PATCH", "POST"]);
  await act(async () => { restore.resolve(json(note("a", 9, "history restored"))); await restoring; });
  expect(result.current.draft).toEqual({ title: "history restored", document: doc("history restored") }); expect(result.current.baseVersion).toBe(9); expect(result.current.resetEpoch).toBe(1);
  await tick(10000); expect(requests).toHaveLength(2);
});
it("new editor callback during restore retains local draft/old base and rejects instead of adopting restored version", async () => {
  vi.useFakeTimers(); const restore = deferred(); const { result, requests } = setup(async () => json(snapshot(1), 201));
  let restoring!: Promise<Note>; act(() => { restoring = result.current.restoreFromServer(async () => await (await restore.promise).json() as Note); }); await tick(0);
  act(() => result.current.changeDraft({ title: "late IME/upload callback", document: doc("latest local") })); await tick(2000);
  expect(requests).toHaveLength(1);
  await act(async () => { restore.resolve(json(note("a", 9, "restored"))); await expect(restoring).rejects.toThrow("仍有新编辑"); });
  expect(result.current.draft).toEqual({ title: "late IME/upload callback", document: doc("latest local") }); expect(result.current.baseVersion).toBe(1); expect(result.current.resetEpoch).toBe(0);
  expect(result.current.status).toBe("conflict"); expect(result.current.serverUpdated).toBe(true); await tick(10000); expect(requests).toHaveLength(1);
});
it("restore failure keeps confirmed base and draft; conflict prevents restore callback before history", async () => {
  vi.useFakeTimers(); const { result, requests } = setup(async () => json(snapshot(1), 201));
  await act(async () => { await expect(result.current.restoreFromServer(async () => { throw new Error("restore response lost"); })).rejects.toThrow("response lost"); });
  expect(result.current.status).toBe("error"); expect(result.current.baseVersion).toBe(1);
  act(() => result.current.changeDraft({ title: "still recoverable" })); await tick(10000); expect(requests).toHaveLength(1);
  const command = vi.fn(async () => note("a", 9, "restored"));
  await act(async () => { await expect(result.current.restoreFromServer(command)).rejects.toThrow("response lost"); }); expect(command).not.toHaveBeenCalled();
});
it("flush calls during a structural command share a barrier and wait for its confirmed version", async () => {
  const move = deferred(); const { result } = setup(async () => json(snapshot(1), 201));
  let moving!: Promise<unknown>; act(() => { moving = result.current.runStructuralMutation(async () => { await move.promise; return note(); }); });
  let one!: Promise<unknown>; let two!: Promise<unknown>;
  act(() => { one = result.current.flush(); two = result.current.flush(); }); expect(one).toBe(two);
  await act(async () => { move.resolve(json({})); await moving; await one; }); expect(result.current.baseVersion).toBe(1);
});
it("same JSON content with server object-key order is a no-op draft and valid structural acknowledgement", async () => {
  vi.useFakeTimers(); const { result, requests } = setup(async () => json(snapshot(1), 201));
  const reordered = { blocks: [{ content: [{ text: "initial", type: "text" as const }], type: "paragraph" as const, id: "p" }] };
  act(() => result.current.changeDraft({ document: reordered }));
  expect(result.current.dirty).toBe(false); await tick(); expect(requests).toHaveLength(0);
  await act(async () => { await result.current.runStructuralMutation(async () => ({ ...note("a", 2), document: reordered })); });
  expect(result.current.baseVersion).toBe(2); expect(result.current.status).toBe("saved");
});
