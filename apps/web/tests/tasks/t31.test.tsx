import { File as NodeFile } from "node:buffer";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { createBrowserHistory, createMemoryHistory } from "@tanstack/react-router";
import { afterAll, afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import type { components } from "@knoted/api-client";
import App from "../../src/App";
import { createAppRouter, safeReturnTo } from "../../src/lib/router";
import { queryKeys } from "../../src/lib/query";
import { UiProvider } from "../../src/lib/ui-provider";
import { pasteHtml } from "../fixtures/editor-clipboard";

type Note = components["schemas"]["Note"];
const id = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
const doc = (text: string): Note["document"] => ({ blocks: [{ id: "paragraph", type: "paragraph", content: [{ type: "text", text }] }] });
const note = (n: number, parent: number | null = null): Note => ({ id: id(n), title: `笔记${n}`, parent_id: parent === null ? null : id(parent), position: n - 1, document: doc(`正文${n}`), content_version: 1, format_version: 1, created_at: "2026-11-17T00:00:00Z", updated_at: "2026-11-17T00:00:00Z", deleted_at: null });
const json = (data: unknown, status = 200) => new Response(JSON.stringify(data), { status, headers: { "Content-Type": "application/json" } });
const failure = (status: number) => json({ code: "FIXTURE", message: "fixture failure", current_version: 9 }, status);
const snapshot = (n: Note, key = "snapshot-1") => ({ snapshot_id: key, note_id: n.id, title: n.title, document: n.document, content_version: n.content_version, reason: "flush", created_at: "2026-11-17T00:00:00Z" });
function deferred() { let resolve!: (r: Response) => void; const promise = new Promise<Response>(r => { resolve = r; }); return { promise, resolve }; }
class BrowserRequest extends Request { constructor(input: RequestInfo | URL, init?: RequestInit) { super(typeof input === "string" ? new URL(input, window.location.origin) : input, init); } }
let notes: Note[];
let requests: Request[];
let browserHistory: ReturnType<typeof createBrowserHistory> | undefined;
let intercept: (r: Request, url: URL) => Response | Promise<Response> | undefined;
beforeAll(() => {
  Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
  Object.defineProperty(Range.prototype, "getBoundingClientRect", { configurable: true, value: () => new DOMRect() });
});
afterAll(() => { Reflect.deleteProperty(Range.prototype, "getClientRects"); Reflect.deleteProperty(Range.prototype, "getBoundingClientRect"); });
beforeEach(() => {
  notes = [note(1), note(2)]; requests = []; intercept = () => undefined;
  vi.stubGlobal("Request", BrowserRequest);
  vi.spyOn(window, "scrollTo").mockImplementation(() => {});
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: vi.fn(async () => {}) } });
  vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL) => {
    const r = input as Request; const url = new URL(r.url); requests.push(r.clone());
    const overridden = intercept(r, url); if (overridden !== undefined) return overridden;
    const path = url.pathname.replace("/api/v1", "");
    if (path === "/auth/session") return json({ session: { id: "00000000-0000-4000-8000-000000000100", user_id: "00000000-0000-4000-8000-000000000101", current: true, created_at: "2026-11-17T00:00:00Z", last_seen_at: "2026-11-17T00:00:00Z", idle_expires_at: "2026-11-24T00:00:00Z", absolute_expires_at: "2026-12-17T00:00:00Z" }, csrf_token: "fixture-csrf" });
    if (path === "/auth/logout") return new Response(null, { status: 204 });
    if (path === "/notes" && r.method === "GET") return json({ items: notes, next_cursor: null });
    if (path === "/notes" && r.method === "POST") { const body = await r.json(); const created = { ...note(99), ...body }; notes.push(created); return json(created, 201); }
    if (path === "/tags" || path === "/links" || path.endsWith("/tags")) return json({ items: [], next_cursor: null });
    const current = notes.find(n => path === `/notes/${n.id}` || path.startsWith(`/notes/${n.id}/`));
    if (!current) return failure(404);
    if (path.endsWith("/snapshots") && r.method === "GET") return json({ items: [snapshot({ ...current, title: "历史标题", document: doc("历史正文") })], next_cursor: null });
    if (path.endsWith("/snapshots") && r.method === "POST") return json(snapshot(current), 201);
    if (path.includes("/snapshots/") && path.endsWith("/restore")) { current.title = "历史标题"; current.document = doc("历史正文"); current.content_version++; return json(current); }
    if (r.method === "PATCH") { const { expected_version, ...changes } = await r.json(); if (expected_version !== current.content_version) return failure(409); Object.assign(current, changes, { content_version: current.content_version + 1 }); return json(current); }
    if (path.endsWith("/move")) {
      const body = await r.json(); const siblings = notes.filter(n => n.id !== current.id && n.parent_id === body.parent_id).sort((a, b) => a.position - b.position);
      siblings.splice(Math.min(body.position, siblings.length), 0, current);
      current.parent_id = body.parent_id; current.content_version++;
      siblings.forEach((n, index) => { n.position = index; }); return json(current);
    }
    if (r.method === "DELETE") { notes = notes.filter(n => n.id !== current.id); return new Response(null, { status: 204 }); }
    return json(current);
  }));
});
afterEach(() => { browserHistory?.destroy(); browserHistory = undefined; vi.useRealTimers(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });
async function mount(path = `/notes/${id(1)}`, browser = false) {
  if (browser) { window.history.replaceState({}, "", path); browserHistory = createBrowserHistory(); }
  const router = createAppRouter({ history: browserHistory ?? createMemoryHistory({ initialEntries: [path] }) });
  render(<UiProvider><App router={router} /></UiProvider>);
  await act(async () => { await router.load(); });
  return { router, bundle: router.options.context! };
}
const calls = (method: string, suffix: string) => requests.filter(r => r.method === method && new URL(r.url).pathname.endsWith(suffix));
async function title() { return screen.findByRole("textbox", { name: "笔记标题" }); }
async function selectNoteAction(name: string) {
  fireEvent.click(await screen.findByRole("button", { name: "更多笔记操作" }));
  fireEvent.click(await screen.findByRole("menuitem", { name }));
}
async function openAssociations() {
  fireEvent.click(screen.getByRole("button", { name: "标签与关联" }));
  return screen.findByRole("dialog", { name: "标签与关联" });
}


it("clipboard rejection keeps real editor, draft copy and navigation flush consistent", async () => {
  const { router } = await mount(); await title();
  const pane = screen.getByRole("region", { name: "当前笔记" });
  const editable = pane.querySelector<HTMLElement>('[contenteditable="true"]')!;
  editable.focus();
  pasteHtml(editable, '<p><a href="https://example.com/#part#two">中文链接</a></p>', "中文链接");
  expect(await within(pane).findByText(/未应用此次编辑/)).toHaveTextContent(/未应用.*原内容已保留/);
  expect(editable).toHaveTextContent("正文1"); expect(editable).not.toHaveTextContent("中文链接");
  pasteHtml(editable, "<p>后续正文</p>", "后续正文");
  await waitFor(() => expect(editable).toHaveTextContent("后续正文"));
  expect(pane.querySelector('[contenteditable="true"]')).toBe(editable);
  await selectNoteAction("复制当前草稿");
  expect(navigator.clipboard.writeText).toHaveBeenCalledWith(expect.stringContaining("后续正文"));
  expect(navigator.clipboard.writeText).not.toHaveBeenCalledWith(expect.stringContaining("中文链接"));
  fireEvent.click(screen.getByRole("treeitem", { name: "笔记2" }));
  await waitFor(() => expect(router.state.location.pathname).toBe(`/notes/${id(2)}`));
  const written = await calls("PATCH", id(1))[0]!.json();
  expect(JSON.stringify(written.document)).toContain("后续正文");
  expect(JSON.stringify(written.document)).not.toContain("#part#two");
  expect(calls("POST", `/notes/${id(1)}/snapshots`)).toHaveLength(1);
  fireEvent.click(screen.getByRole("treeitem", { name: "笔记1" }));
  await waitFor(() => expect(screen.getByRole("textbox", { name: "笔记标题" })).toHaveValue("笔记1"));
  expect(screen.getByRole("region", { name: "当前笔记" })).toHaveTextContent("后续正文");
});

it("t31_tree_all_cursor_pages_and_real_branch reads 51+ notes before constructing the real Tree", async () => {
  notes = Array.from({ length: 50 }, (_, index) => note(index + 1)); const child = note(51, 1);
  intercept = (r, url) => url.pathname === "/api/v1/notes" && r.method === "GET"
    ? json(url.searchParams.get("cursor") === "opaque-page-2" ? { items: [child], next_cursor: null } : { items: notes, next_cursor: "opaque-page-2" }) : undefined;
  await mount("/notes");
  const root = await screen.findByRole("treeitem", { name: "笔记1" });
  expect(root).toHaveAttribute("aria-expanded", "false");
  fireEvent.keyDown(root, { key: "ArrowRight" });
  expect(await screen.findByRole("treeitem", { name: "笔记51" })).toBeInTheDocument();
  const pages = calls("GET", "/notes"); expect(pages).toHaveLength(2);
  expect(new URL(pages[1]!.url).searchParams.get("cursor")).toBe("opaque-page-2");
  expect(pages.every(r => !new URL(r.url).searchParams.has("parent_id"))).toBe(true);
});
it("t31_registered_routes_and_url_safety protects the real notes destination", async () => {
  expect(safeReturnTo(`/notes/${id(1)}`)).toBe(`/notes/${id(1)}`);
  for (const value of ["/notes-evil", "/notes/invalid", `/notes/${id(1)}/evil`, "//evil", "/notes/%2f%2fevil", "/notes\\evil"]) expect(safeReturnTo(value)).toBe("/");
  intercept = (_r, u) => u.pathname.endsWith("/auth/session") ? failure(401) : undefined;
  const { router } = await mount();
  await screen.findByRole("button", { name: "使用 Passkey 登录" }); expect(router.state.location.pathname).toBe("/login");
  expect(router.state.location.search).toEqual({ redirect: `/notes/${id(1)}` });
});
it("t31_navigation_barrier_and_draft_failures keeps route/editor selected until mounted save and history finish", async () => {
  const pending = deferred(); intercept = (r) => r.method === "PATCH" ? pending.promise : undefined;
  const { router } = await mount(); const input = await title();
  fireEvent.change(input, { target: { value: "留稿" } });
  fireEvent.click(await screen.findByRole("treeitem", { name: "笔记2" }));
  await waitFor(() => expect(calls("PATCH", id(1))).toHaveLength(1));
  expect(router.state.location.pathname).toBe(`/notes/${id(1)}`); expect(input).toHaveValue("留稿");
  await act(async () => pending.resolve(failure(503)));
  expect(await screen.findByText(/无法离开/)).toBeInTheDocument(); expect(router.state.location.pathname).toBe(`/notes/${id(1)}`);
});
it("t31_clean_refetch_never_upgrades_old_editor_base and Query clear retains initialized editor", async () => {
  const { bundle } = await mount(); const input = await title(); const editor = screen.getByRole("region", { name: "当前笔记" }).querySelector('[contenteditable="true"]');
  act(() => bundle.queryClient.setQueryData(queryKeys.notes.detail(id(1)), { ...note(1), content_version: 9, title: "远端标题" }));
  expect(input).toHaveValue("笔记1"); expect(screen.getByText(/服务器内容已更新/)).toBeInTheDocument();
  act(() => bundle.queryClient.clear()); expect(input).toHaveValue("笔记1");
  expect(screen.getByRole("region", { name: "当前笔记" }).querySelector('[contenteditable="true"]')).toBe(editor);
});
it("history replaces the writing area and gives focus back without remounting the draft editor", async () => {
  notes[0]!.document = doc("长正文".repeat(500));
  await mount();
  const currentTitle = await title();
  const pane = screen.getByRole("region", { name: "当前笔记" });
  const editable = pane.querySelector('[contenteditable="true"]');
  fireEvent.change(currentTitle, { target: { value: "还在编辑的标题" } });

  const history = screen.getByRole("button", { name: "历史版本" });
  fireEvent.click(history);
  const versions = await screen.findByRole("region", { name: "笔记历史" });
  expect(within(pane).queryByRole("region", { name: "笔记编辑器" })).not.toBeInTheDocument();
  expect(editable).not.toBeVisible();
  fireEvent.click(within(versions).getByRole("button", { name: "关闭历史" }));
  expect(history).toHaveFocus();
  expect(pane.querySelector('[contenteditable="true"]')).toBe(editable);
  expect(currentTitle).toHaveValue("还在编辑的标题");
});

it("directory drawer stays open on a blocked note switch while the draft remains active", async () => {
  intercept = r => r.method === "PATCH" ? failure(503) : undefined;
  const { router } = await mount();
  const input = await title();
  fireEvent.change(input, { target: { value: "目录切换仍保留" } });

  fireEvent.click(screen.getByRole("button", { name: "目录" }));
  const directory = await screen.findByRole("dialog", { name: "笔记目录" });
  fireEvent.click(await within(directory).findByRole("treeitem", { name: "笔记2" }));
  expect(await within(directory).findByText(/无法离开/)).toBeInTheDocument();
  expect(router.state.location.pathname).toBe(`/notes/${id(1)}`);
  expect(input).toHaveValue("目录切换仍保留");
});

it("t31_history_readonly_and_pagination uses a genuinely noneditable NoteEditor", async () => {
  await mount(); await title(); fireEvent.click(screen.getByRole("button", { name: "历史版本" }));
  fireEvent.click(await screen.findByRole("button", { name: /预览.*历史标题/ }));
  const preview = await screen.findByRole("region", { name: "历史预览" });
  expect(preview.querySelector('[contenteditable="false"]')).toBeInTheDocument();
  expect(within(preview).queryByRole("button")).not.toBeInTheDocument(); expect(within(preview).getByText("历史正文")).toBeInTheDocument();
  expect(calls("PATCH", id(1))).toHaveLength(0);
});

it.each([503, 409])("t31_1_failed_move_never_drifts (%i)", async status => {
  const pending = deferred(); intercept = (_r, url) => url.pathname.endsWith("/move") ? pending.promise : undefined;
  const { router } = await mount(); await title();
  const treeBefore = screen.getAllByRole("treeitem").map(item => item.textContent);
  await selectNoteAction("移动笔记");
  fireEvent.click(await screen.findByRole("button", { name: "确认移动" }));
  await waitFor(() => expect(calls("POST", "/move")).toHaveLength(1));
  expect(screen.getAllByRole("treeitem", { hidden: true }).map(item => item.textContent)).toEqual(treeBefore);
  await act(async () => pending.resolve(failure(status)));
  await waitFor(() => expect(screen.getAllByText("fixture failure").length).toBeGreaterThan(0));
  expect(screen.getAllByRole("treeitem", { hidden: true }).map(item => item.textContent)).toEqual(treeBefore);
  expect(router.state.location.pathname).toBe(`/notes/${id(1)}`);
});
it("t31_current_move_advances_own_base_without_remount uses fresh save version and rereads tree", async () => {
  await mount(); const input = await title(); const editor = screen.getByRole("region", { name: "当前笔记" }).querySelector('[contenteditable="true"]');
  fireEvent.change(input, { target: { value: "移动前新稿" } });
  await selectNoteAction("移动笔记"); fireEvent.click(await screen.findByRole("button", { name: "确认移动" }));
  await waitFor(() => expect(calls("POST", "/move")).toHaveLength(1));
  expect(await calls("POST", "/move")[0]!.json()).toEqual({ expected_version: 2, parent_id: null, position: 1 });
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  expect(screen.getByRole("region", { name: "当前笔记" }).querySelector('[contenteditable="true"]')).toBe(editor);
  expect(calls("GET", "/notes").length).toBeGreaterThanOrEqual(2);
  expect(screen.getAllByRole("treeitem").map(item => item.textContent)).toEqual(["笔记2", "移动前新稿"]);
  fireEvent.change(input, { target: { value: "移动后新稿" } }); fireEvent.click(screen.getByRole("button", { name: "保存并记录历史" }));
  await waitFor(() => expect(calls("PATCH", id(1))).toHaveLength(2));
  expect(await calls("PATCH", id(1))[1]!.json()).toMatchObject({ expected_version: 3, title: "移动后新稿" });
});
it("t31_2_history_restore_new_document_and_version flushes then atomically replaces title/body/base", async () => {
  const pending = deferred(); intercept = (_r, url) => url.pathname.includes("/snapshots/") && url.pathname.endsWith("/restore") ? pending.promise : undefined;
  await mount(); const input = await title(); fireEvent.change(input, { target: { value: "当前新稿" } });
  fireEvent.click(screen.getByRole("button", { name: "历史版本" })); fireEvent.click(await screen.findByRole("button", { name: /预览.*历史标题/ }));
  const oldEditor = screen.getByRole("region", { name: "当前笔记" }).querySelector('[contenteditable="true"]');
  fireEvent.click(screen.getByRole("button", { name: "恢复此历史版本" })); fireEvent.click(await screen.findByRole("button", { name: "确认恢复" }));
  await waitFor(() => expect(calls("POST", "/snapshot-1/restore")).toHaveLength(1));
  expect(await calls("POST", "/snapshot-1/restore")[0]!.json()).toEqual({ expected_version: 2 });
  expect(input).toHaveValue("当前新稿");
  const restored = { ...note(1), title: "恢复标题", document: doc("恢复正文"), content_version: 3 }; notes[0] = restored;
  await act(async () => pending.resolve(json(restored)));
  await waitFor(() => expect(input).toHaveValue("恢复标题")); expect(screen.getByText("恢复正文")).toBeInTheDocument();
  await waitFor(() => expect(input).toHaveFocus());
  expect(screen.getByRole("region", { name: "当前笔记" }).querySelector('[contenteditable="true"]')).not.toBe(oldEditor);
  fireEvent.change(input, { target: { value: "恢复后编辑" } }); fireEvent.click(screen.getByRole("button", { name: "保存并记录历史" }));
  await waitFor(() => expect(calls("PATCH", id(1))).toHaveLength(2)); expect(await calls("PATCH", id(1))[1]!.json()).toMatchObject({ expected_version: 3 });
});
it.each([409, 503])("failed history restore %i retains the current title/document", async status => {
  intercept = (_r, u) => u.pathname.includes("/snapshots/") && u.pathname.endsWith("/restore") ? failure(status) : undefined;
  await mount(); const input = await title(); fireEvent.change(input, { target: { value: "恢复失败留稿" } });
  fireEvent.click(screen.getByRole("button", { name: "历史版本" })); fireEvent.click(await screen.findByRole("button", { name: /预览.*历史标题/ }));
  fireEvent.click(screen.getByRole("button", { name: "恢复此历史版本" })); fireEvent.click(await screen.findByRole("button", { name: "确认恢复" }));
  await waitFor(() => expect(calls("POST", "/snapshot-1/restore")).toHaveLength(1));
  expect(input).toHaveValue("恢复失败留稿"); expect(screen.getByText("正文1")).toBeInTheDocument(); expect(await screen.findByRole("button", { name: "确认恢复" })).toBeEnabled();
});
it("t31_logout_preflight_order does not revoke authentication before safe mounted save/history/navigation", async () => {
  const pending = deferred(); intercept = (r) => r.method === "PATCH" ? pending.promise : undefined;
  const { router } = await mount(); const input = await title(); fireEvent.change(input, { target: { value: "退出前保存" } });
  fireEvent.click(screen.getByRole("button", { name: "退出登录" }));
  await waitFor(() => expect(calls("PATCH", id(1))).toHaveLength(1)); expect(calls("POST", "/auth/logout")).toHaveLength(0);
  const saved = { ...note(1), title: "退出前保存", content_version: 2 }; notes[0] = saved;
  await act(async () => pending.resolve(json(saved)));
  await waitFor(() => expect(router.state.location.pathname).toBe("/login"));
  const order = requests.map(r => `${r.method} ${new URL(r.url).pathname}`);
  expect(order.indexOf(`POST /api/v1/notes/${id(1)}/snapshots`)).toBeLessThan(order.indexOf("POST /api/v1/auth/logout"));
});
it.each([503, 409])("failed logout flush %i leaves draft and zero logout requests even if navigate resolves", async status => {
  intercept = r => r.method === "PATCH" ? failure(status) : undefined;
  const { router } = await mount(); const input = await title(); fireEvent.change(input, { target: { value: "退出失败留稿" } });
  fireEvent.click(screen.getByRole("button", { name: "退出登录" }));
  await screen.findByText(/无法安全离开笔记/); expect(calls("POST", "/auth/logout")).toHaveLength(0);
  expect(router.state.location.pathname).toBe(`/notes/${id(1)}`); expect(input).toHaveValue("退出失败留稿");
});
it("t31_3_parent_delete_gate_and_real_undo uses receipt CAS, root fallback and a fresh restored session", async () => {
  notes = [note(1), note(2, 1)]; let deletedChild: Note | undefined;
  intercept = (r, u) => {
    if (r.method === "DELETE" && u.pathname.endsWith(id(1))) return failure(409);
    if (r.method === "DELETE" && u.pathname.endsWith(id(2))) { deletedChild = { ...notes.find(n => n.id === id(2))! }; notes = notes.filter(n => n.id !== id(2)); return new Response(null, { status: 204 }); }
    if (u.pathname.endsWith(`/notes/${id(2)}/restore`)) return r.clone().json().then(body => {
      if (!body.restore_parent) return failure(409);
      const restored = { ...deletedChild!, parent_id: null, content_version: 3 }; notes.push(restored); return json(restored);
    });
    return undefined;
  };
  const { router } = await mount(); await title(); await selectNoteAction("删除笔记");
  expect(await screen.findByText(/仍有子页/)).toBeInTheDocument(); fireEvent.click(screen.getByRole("button", { name: "确认删除" }));
  await waitFor(() => expect(calls("DELETE", id(1))).toHaveLength(1)); expect(router.state.location.pathname).toBe(`/notes/${id(1)}`);
  fireEvent.click(screen.getByRole("button", { name: "取消" }));
  await act(async () => { await router.navigate({ to: "/notes/$noteId", params: { noteId: id(2) } }); });
  await selectNoteAction("删除笔记"); fireEvent.click(await screen.findByRole("button", { name: "确认删除" }));
  await waitFor(() => expect(router.state.location.pathname).toBe("/notes"));
  expect(calls("POST", `/notes/${id(2)}/snapshots`)).toHaveLength(1);
  fireEvent.click(await screen.findByRole("button", { name: "恢复本次删除 笔记2" }));
  expect(await screen.findByRole("button", { name: "恢复到根 笔记2" })).toBeInTheDocument();
  expect(await calls("POST", `/notes/${id(2)}/restore`)[0]!.json()).toEqual({ expected_version: 2 });
  fireEvent.click(screen.getByRole("button", { name: "恢复到根 笔记2" }));
  await waitFor(() => expect(router.state.location.pathname).toBe(`/notes/${id(2)}`));
  expect(await calls("POST", `/notes/${id(2)}/restore`)[1]!.json()).toEqual({ expected_version: 2, restore_parent: { parent_id: null } });
  fireEvent.change(await title(), { target: { value: "新恢复会话" } }); fireEvent.click(screen.getByRole("button", { name: "保存并记录历史" }));
  await waitFor(() => expect(calls("PATCH", id(2))).toHaveLength(1)); expect(await calls("PATCH", id(2))[0]!.json()).toMatchObject({ expected_version: 3 });
  expect(requests.some(r => new URL(r.url).pathname.includes("/trash"))).toBe(false);
});
it("t31_notes_401_keeps_initialized_draft on associations and stops a pending dirty timer", async () => {
  const pending = deferred(); intercept = (_r, u) => u.pathname === "/api/v1/links" ? pending.promise : undefined;
  const { router, bundle } = await mount(); const input = await title(); fireEvent.change(input, { target: { value: "过期也保留" } });
  await act(async () => pending.resolve(failure(401)));
  await screen.findByText("会话已失效，本地草稿已保留");
  await new Promise(resolve => setTimeout(resolve, 1100));
  expect(calls("PATCH", id(1))).toHaveLength(0); expect(router.state.location.pathname).toBe(`/notes/${id(1)}`); expect(input).toHaveValue("过期也保留");
  expect(screen.getByRole("link", { name: "在新标签页重新登录" })).toHaveAttribute("target", "_blank");
  await selectNoteAction("复制当前草稿"); expect(navigator.clipboard.writeText).toHaveBeenCalledWith(expect.stringContaining("过期也保留"));
  expect(bundle.queryClient.getQueryData(queryKeys.auth.session())).toBeUndefined();
});
it("orphaned complete-tree responses surface a retry error without unmounting the local editor", async () => {
  notes = [note(1), note(2, 999)]; await mount(); const input = await title();
  expect(await screen.findByText(/目录.*不完整/)).toBeInTheDocument(); expect(input).toHaveValue("笔记1"); expect(screen.queryByRole("treeitem", { name: "笔记2" })).not.toBeInTheDocument();
});

it("initial notes index tree401 has no draft and redirects to the existing login flow", async () => {
  intercept = (r, u) => r.method === "GET" && u.pathname === "/api/v1/notes" ? failure(401) : undefined;
  const { router } = await mount("/notes"); await screen.findByRole("button", { name: "使用 Passkey 登录" });
  expect(router.state.location.pathname).toBe("/login"); expect(router.state.location.search).toEqual({ redirect: "/notes" });
});
it("history pagination passes opaque cursor, disables while loading, and resets selection/note", async () => {
  const second = deferred(); intercept = (r, u) => r.method === "GET" && u.pathname.endsWith("/snapshots") ? u.searchParams.has("cursor") ? second.promise : json({ items: [snapshot({ ...notes[0]!, title: "分页历史" })], next_cursor: "history-opaque" }) : undefined;
  const { router } = await mount(); await title(); fireEvent.click(screen.getByRole("button", { name: "历史版本" }));
  fireEvent.click(await screen.findByRole("button", { name: /预览.*分页历史/ }));
  fireEvent.click(screen.getByRole("button", { name: "下一页历史" }));
  expect(screen.getByRole("button", { name: "下一页历史" })).toBeDisabled(); expect(screen.getByRole("button", { name: "上一页历史" })).toBeDisabled();
  expect(screen.queryByRole("region", { name: "历史预览" })).not.toBeInTheDocument();
  await waitFor(() => expect(calls("GET", "/snapshots")).toHaveLength(2));
  expect(new URL(calls("GET", "/snapshots")[1]!.url).searchParams.get("cursor")).toBe("history-opaque");
  await act(async () => second.resolve(json({ items: [], next_cursor: null })));
  expect(await screen.findByText("暂无历史版本")).toBeInTheDocument(); expect(screen.getByRole("button", { name: "上一页历史" })).toBeEnabled();
  await act(async () => { await router.navigate({ to: "/notes/$noteId", params: { noteId: id(2) } }); });
  fireEvent.click(await screen.findByRole("button", { name: "历史版本" })); await screen.findByRole("button", { name: /预览.*分页历史/ });
  const last = calls("GET", "/snapshots").at(-1)!; expect(new URL(last.url).searchParams.has("cursor")).toBe(false);
  expect(screen.getByRole("button", { name: "上一页历史" })).toBeDisabled();
});
it("successful navigation awaits deferred history while the same editor stays mounted", async () => {
  const pending = deferred(); intercept = (r, u) => r.method === "POST" && u.pathname.endsWith(`/notes/${id(1)}/snapshots`) ? pending.promise : undefined;
  const { router } = await mount(); const input = await title();
  fireEvent.click(screen.getByRole("treeitem", { name: "笔记2" }));
  await waitFor(() => expect(calls("POST", "/snapshots")).toHaveLength(1));
  expect(router.state.location.pathname).toBe(`/notes/${id(1)}`); expect(input).toBeInTheDocument();
  expect(screen.getByRole("treeitem", { name: "笔记1" })).toHaveAttribute("aria-selected", "true");
  await act(async () => pending.resolve(json(snapshot(notes[0]!), 201)));
  await waitFor(() => expect(router.state.location.pathname).toBe(`/notes/${id(2)}`)); expect(await title()).toHaveValue("笔记2");
});
it("sidebar and back navigation share the mounted notes flush barrier", async () => {
  const { router } = await mount(`/notes/${id(1)}`, true); await title();
  await act(async () => { await router.navigate({ to: "/notes/$noteId", params: { noteId: id(2) } }); });
  await waitFor(() => expect(screen.getByRole("textbox", { name: "笔记标题" })).toHaveValue("笔记2"));
  const pending = deferred(); intercept = (r, u) => r.method === "POST" && u.pathname.endsWith(`/notes/${id(2)}/snapshots`) ? pending.promise : undefined;
  act(() => router.history.back());
  await waitFor(() => expect(calls("POST", `/notes/${id(2)}/snapshots`)).toHaveLength(1)); expect(router.state.location.pathname).toBe(`/notes/${id(2)}`);
  await act(async () => pending.resolve(failure(503)));
  await screen.findByText(/无法离开/); expect(router.state.location.pathname).toBe(`/notes/${id(2)}`);
  fireEvent.click(screen.getByRole("button", { name: "凭据与会话" }));
  await waitFor(() => expect(calls("POST", `/notes/${id(2)}/snapshots`)).toHaveLength(2)); expect(router.state.location.pathname).toBe(`/notes/${id(2)}`);
});
it("blocked logout becomes retryable, then explicit save/history and safe location precede revocation", async () => {
  let fails = true; intercept = r => r.method === "PATCH" && fails ? failure(503) : undefined;
  const { router } = await mount(); fireEvent.change(await title(), { target: { value: "可重试退出" } }); fireEvent.click(screen.getByRole("button", { name: "退出登录" }));
  await screen.findByText(/无法安全离开笔记/); expect(screen.getByRole("button", { name: "退出登录" })).toBeEnabled(); expect(calls("POST", "/auth/logout")).toHaveLength(0);
  fails = false; fireEvent.click(screen.getByRole("button", { name: "保存并记录历史" }));
  await waitFor(() => expect(calls("POST", "/snapshots")).toHaveLength(1));
  await waitFor(() => expect(screen.getByRole("button", { name: "保存并记录历史" })).toBeEnabled());
  let locationAtLogout = ""; intercept = (_r, u) => { if (u.pathname.endsWith("/auth/logout")) locationAtLogout = router.state.location.pathname; return undefined; };
  fireEvent.click(screen.getByRole("button", { name: "退出登录" })); await screen.findByRole("button", { name: "使用 Passkey 登录" });
  expect(locationAtLogout).toBe("/"); expect(calls("POST", "/auth/logout")).toHaveLength(1);
});
it("snapshot401 while a structural command waits prevents MOVE and keeps the old session", async () => {
  const pending = deferred(); intercept = (r, u) => r.method === "POST" && u.pathname.endsWith("/snapshots") ? pending.promise : undefined;
  const { router } = await mount(); const input = await title(); fireEvent.change(input, { target: { value: "结构命令失效留稿" } });
  await selectNoteAction("移动笔记"); fireEvent.click(await screen.findByRole("button", { name: "确认移动" }));
  await waitFor(() => expect(calls("POST", "/snapshots")).toHaveLength(1)); await act(async () => pending.resolve(failure(401)));
  await screen.findByText("会话已失效，本地草稿已保留"); expect(calls("POST", "/move")).toHaveLength(0);
  expect(router.state.location.pathname).toBe(`/notes/${id(1)}`); expect(input).toHaveValue("结构命令失效留稿");
});
it("autosave401 preserves local editor after cache removal and explicit reauthentication retries the original CAS", async () => {
  let failWrite = true; intercept = r => r.method === "PATCH" && failWrite ? failure(401) : undefined;
  const { router } = await mount(); const input = await title(); fireEvent.change(input, { target: { value: "认证后重试" } });
  fireEvent.click(screen.getByRole("button", { name: "保存并记录历史" })); await screen.findByText("会话已失效，本地草稿已保留");
  expect(router.state.location.pathname).toBe(`/notes/${id(1)}`); expect(input).toHaveValue("认证后重试");
  failWrite = false; fireEvent.click(screen.getByRole("button", { name: "已重新登录，重试原版本" }));
  await waitFor(() => expect(calls("PATCH", id(1))).toHaveLength(2));
  expect(await calls("PATCH", id(1))[1]!.json()).toMatchObject({ expected_version: 1, title: "认证后重试" });
  await waitFor(() => expect(input).toBeEnabled());
});
it("tree loading failure does not show partial nodes and retry starts from the first cursor", async () => {
  let fail = true; intercept = (r, u) => {
    if (r.method !== "GET" || u.pathname !== "/api/v1/notes") return undefined;
    if (!u.searchParams.has("cursor")) return json({ items: [note(1)], next_cursor: "next" });
    return fail ? failure(503) : json({ items: [note(2, 1)], next_cursor: null });
  };
  await mount(); await title(); await screen.findByRole("button", { name: "重试目录" }); expect(screen.queryByRole("treeitem")).not.toBeInTheDocument();
  fail = false; fireEvent.click(screen.getByRole("button", { name: "重试目录" })); await screen.findByRole("treeitem", { name: "笔记1" });
  const pages = calls("GET", "/notes"); expect(pages).toHaveLength(4); expect(new URL(pages[2]!.url).searchParams.has("cursor")).toBe(false);
});
it("t31_editor_all_business_controls_mounted uses actual menus, upload, code and associations", async () => {
  notes[0]!.document = { blocks: [{ id: "code", type: "codeBlock", language: "rust", content: [{ type: "text", text: "fn main() {}" }] }] };
  await mount(); await title();
  expect(await screen.findByRole("group", { name: "笔记图片上传" })).toBeInTheDocument();
  expect(screen.getByRole("combobox", { name: "代码语言" })).toBeInTheDocument();
  expect(within(await openAssociations()).getByRole("region", { name: "笔记标签与资料关联" })).toBeInTheDocument();
  expect(requests.some(r => new URL(r.url).pathname.endsWith(`/entities/note/${id(1)}/tags`))).toBe(true);
  expect(requests.some(r => new URL(r.url).pathname.endsWith("/links"))).toBe(true);
});

it("association401 during a successful pending save prevents the later snapshot write", async () => {
  const links = deferred(); const patch = deferred(); intercept = (r, u) => u.pathname.endsWith("/links") ? links.promise : r.method === "PATCH" ? patch.promise : undefined;
  await mount(); const input = await title(); fireEvent.change(input, { target: { value: "等待保存时过期" } }); fireEvent.click(screen.getByRole("button", { name: "保存并记录历史" }));
  await waitFor(() => expect(calls("PATCH", id(1))).toHaveLength(1)); await act(async () => links.resolve(failure(401)));
  await screen.findByText("会话已失效，本地草稿已保留");
  const saved = { ...note(1), title: "等待保存时过期", content_version: 2 }; notes[0] = saved;
  await act(async () => patch.resolve(json(saved)));
  await waitFor(() => expect(screen.getByText(/历史保存失败/)).toBeInTheDocument());
  expect(calls("POST", "/snapshots")).toHaveLength(0); expect(input).toHaveValue("等待保存时过期");
});
it("association401 during pending history prevents a subsequently queued MOVE", async () => {
  const links = deferred(); const history = deferred(); intercept = (r, u) => u.pathname.endsWith("/links") ? links.promise : r.method === "POST" && u.pathname.endsWith("/snapshots") ? history.promise : undefined;
  await mount(); const input = await title(); await selectNoteAction("移动笔记"); fireEvent.click(await screen.findByRole("button", { name: "确认移动" }));
  await waitFor(() => expect(calls("POST", "/snapshots")).toHaveLength(1)); await act(async () => links.resolve(failure(401)));
  await screen.findByText("会话已失效，本地草稿已保留"); await act(async () => history.resolve(json(snapshot(notes[0]!), 201)));
  await waitFor(() => expect(screen.getAllByText(/会话已失效，草稿已保留/).length).toBeGreaterThan(0));
  expect(calls("POST", "/move")).toHaveLength(0); expect(input).toHaveValue("笔记1");
});
it("association401 during pending history prevents a subsequently queued DELETE", async () => {
  const links = deferred(); const history = deferred(); intercept = (r, u) => u.pathname.endsWith("/links") ? links.promise : r.method === "POST" && u.pathname.endsWith("/snapshots") ? history.promise : undefined;
  const { router } = await mount(); const input = await title(); await selectNoteAction("删除笔记"); fireEvent.click(await screen.findByRole("button", { name: "确认删除" }));
  await waitFor(() => expect(calls("POST", "/snapshots")).toHaveLength(1)); await act(async () => links.resolve(failure(401)));
  await screen.findByText("会话已失效，本地草稿已保留"); await act(async () => history.resolve(json(snapshot(notes[0]!), 201)));
  await waitFor(() => expect(screen.getByRole("button", { name: "确认删除" })).toBeEnabled());
  expect(calls("DELETE", id(1))).toHaveLength(0); expect(router.state.location.pathname).toBe(`/notes/${id(1)}`); expect(input).toHaveValue("笔记1");
});

it("removing a child allows its parent DELETE, and each confirmed deletion leaves an independent receipt", async () => {
  notes = [note(1), note(2, 1)]; intercept = (r, u) => r.method === "DELETE" && notes.some(n => n.parent_id === u.pathname.split("/").at(-1)) ? failure(409) : undefined;
  const { router } = await mount(); await title(); await selectNoteAction("删除笔记"); fireEvent.click(await screen.findByRole("button", { name: "确认删除" }));
  await waitFor(() => expect(calls("DELETE", id(1))).toHaveLength(1)); expect(notes).toHaveLength(2);
  fireEvent.click(screen.getByRole("button", { name: "取消" }));
  await act(async () => { await router.navigate({ to: "/notes/$noteId", params: { noteId: id(2) } }); });
  await selectNoteAction("删除笔记"); fireEvent.click(await screen.findByRole("button", { name: "确认删除" }));
  await waitFor(() => expect(router.state.location.pathname).toBe("/notes")); expect(notes.map(n => n.id)).toEqual([id(1)]);
  fireEvent.click(await screen.findByRole("treeitem", { name: "笔记1" }));
  await selectNoteAction("删除笔记"); fireEvent.click(await screen.findByRole("button", { name: "确认删除" }));
  await waitFor(() => expect(router.state.location.pathname).toBe("/notes")); expect(notes).toHaveLength(0);
  expect(screen.getByRole("button", { name: "恢复本次删除 笔记1" })).toBeInTheDocument(); expect(screen.getByRole("button", { name: "恢复本次删除 笔记2" })).toBeInTheDocument();
});
it("confirmed title saves update tree labels without projecting an unsaved draft into the tree", async () => {
  await mount(); const input = await title(); fireEvent.change(input, { target: { value: "树上的已确认标题" } });
  expect(screen.getByRole("treeitem", { name: "笔记1" })).toBeInTheDocument(); expect(screen.queryByRole("treeitem", { name: "树上的已确认标题" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "保存并记录历史" }));
  expect(await screen.findByRole("treeitem", { name: "树上的已确认标题" })).toBeInTheDocument();
});

it("association tag creation acknowledged after independent401 pauses assignment and retains the created ID for explicit retry", async () => {
  const tagPost = deferred(); const createdTag = { id: id(98), name: "失效后保留标签", version: 1, created_at: "2026-11-17T00:00:00Z", updated_at: "2026-11-17T00:00:00Z" };
  let created = false; let assigned = false; let expired = false;
  intercept = (r, u) => {
    if (r.method === "POST" && u.pathname === "/api/v1/tags") return tagPost.promise;
    if (r.method === "PUT" && u.pathname.endsWith(`/tags/${id(98)}`)) { assigned = true; return new Response(null, { status: 204 }); }
    if (r.method === "GET" && u.pathname.endsWith("/links") && expired) return failure(401);
    if (r.method === "GET" && u.pathname === "/api/v1/tags") return json({ items: created ? [createdTag] : [], next_cursor: null });
    if (r.method === "GET" && u.pathname.endsWith(`/entities/note/${id(1)}/tags`)) return json({ items: assigned ? [createdTag] : [], next_cursor: null });
    return undefined;
  };
  const { bundle, router } = await mount(); const input = await title();
  const associations = await openAssociations();
  fireEvent.click(within(associations).getByRole("button", { name: "添加标签" }));
  const create = await screen.findByRole("button", { name: "创建并添加标签" }); await waitFor(() => expect(create).toBeEnabled());
  fireEvent.change(screen.getByRole("textbox", { name: "新标签名称" }), { target: { value: "失效后保留标签" } }); fireEvent.click(create);
  await waitFor(() => expect(calls("POST", "/tags")).toHaveLength(1));
  expired = true; await act(async () => { await bundle.api.raw.GET("/links", { params: { query: { source_type: "note", source_id: id(1) } } }).catch(() => {}); });
  await screen.findByText("会话已失效，本地草稿已保留"); created = true;
  await act(async () => tagPost.resolve(json(createdTag, 201)));
  expect(calls("PUT", `/tags/${id(98)}`)).toHaveLength(0); expect(assigned).toBe(false);
  expect(input).toHaveValue("笔记1"); expect(router.state.location.pathname).toBe(`/notes/${id(1)}`);
  expect(screen.getByRole("button", { name: "添加已有标签" })).toBeDisabled();
  fireEvent.click(within(associations).getByRole("button", { name: "关闭" }));
  expired = false; fireEvent.click(await screen.findByRole("button", { name: "已重新登录，重试原版本" }));
  await waitFor(() => expect(screen.queryByText("会话已失效，本地草稿已保留")).not.toBeInTheDocument());
  await openAssociations();
  await waitFor(() => expect(screen.getByRole("button", { name: "添加已有标签" })).toBeEnabled());
  fireEvent.click(screen.getByRole("button", { name: "添加已有标签" }));
  await screen.findByRole("button", { name: "移除标签 失效后保留标签" });
  expect(calls("POST", "/tags")).toHaveLength(1); expect(calls("PUT", `/tags/${id(98)}`)).toHaveLength(1);
});

it("returning A→B→A seeds a new real editor from the saved draft, retaining uploaded images and code language on the next edit", async () => {
  const assetId = id(97);
  notes[0]!.document = { blocks: [
    { id: "code-return", type: "codeBlock", language: "rust", content: [{ type: "text", text: "retained code();" }] },
    { id: "paragraph-return", type: "paragraph", content: [{ type: "text", text: "保留的第二段" }] },
  ] };
  intercept = (r, u) => r.method === "POST" && u.pathname === "/api/v1/assets"
    ? json({ id: assetId, mime: "image/png", size: 68, sha256: "a".repeat(64), state: "ready", created_at: "2026-11-17T00:00:00Z" }, 201) : undefined;
  const { bundle, router } = await mount(); await title();
  fireEvent.click(await screen.findByRole("combobox", { name: "代码语言" }));
  const javascript = await screen.findByRole("option", { name: "JavaScript" });
  fireEvent.pointerDown(javascript, { pointerType: "mouse" }); fireEvent.click(javascript);
  const nativeForm = await new BrowserRequest("/", { method: "POST", headers: { "Content-Type": "application/x-www-form-urlencoded" }, body: "" }).formData();
  vi.stubGlobal("FormData", nativeForm.constructor); vi.stubGlobal("File", NodeFile);
  fireEvent.change(screen.getByLabelText("选择文件"), { target: { files: [new File(["public PNG fixture"], "return.png", { type: "image/png" })] } });
  await screen.findByText("上传成功");
  const current = screen.getByRole("region", { name: "当前笔记" });
  const editable = current.querySelector<HTMLElement>('[contenteditable="true"]')!;
  const selection = window.getSelection()!;
  await act(async () => {
    editable.focus();
    const codeText = document.createTreeWalker(current.querySelector("pre code")!, NodeFilter.SHOW_TEXT).nextNode()!;
    const range = document.createRange(); range.setStart(codeText, 2); range.collapse(true);
    selection.removeAllRanges(); selection.addRange(range); document.dispatchEvent(new Event("selectionchange"));
  });
  const anchor = selection.anchorNode; const offset = selection.anchorOffset;
  // Observe the real 1000ms debounce; do not replace a timer already created by the editor.
  await waitFor(() => expect(calls("PATCH", id(1))).toHaveLength(1), { timeout: 2500 });
  await waitFor(() => expect(current.querySelector("p[role=status]")).toHaveTextContent("已保存 · 当前基础版本 2"));
  expect(current.querySelector('[contenteditable="true"]')).toBe(editable);
  expect(selection.anchorNode).toBe(anchor); expect(selection.anchorOffset).toBe(offset);
  const saved = await calls("PATCH", id(1))[0]!.json();
  expect(saved.document.blocks).toEqual(expect.arrayContaining([
    expect.objectContaining({ id: "code-return", type: "codeBlock", language: "javascript" }),
    expect.objectContaining({ type: "image", asset_id: assetId }),
    expect.objectContaining({ id: "paragraph-return", type: "paragraph" }),
  ]));
  act(() => bundle.queryClient.setQueryData(queryKeys.notes.detail(id(1)), { ...structuredClone(notes[0]!), updated_at: "2026-11-17T00:01:00Z" }));
  expect(current.querySelector('[contenteditable="true"]')).toBe(editable);
  expect(selection.anchorNode).toBe(anchor); expect(selection.anchorOffset).toBe(offset);
  await act(async () => { await router.navigate({ to: "/notes/$noteId", params: { noteId: id(2) } }); });
  expect(await title()).toHaveValue("笔记2");
  await act(async () => { await router.navigate({ to: "/notes/$noteId", params: { noteId: id(1) } }); });
  expect(await title()).toHaveValue("笔记1");
  const reopened = screen.getByRole("region", { name: "当前笔记" });
  expect(reopened.querySelector('[contenteditable="true"]')).not.toBe(editable);
  expect(within(reopened).getByRole("img", { name: "笔记图片" })).toHaveAttribute("src", `/api/v1/assets/${assetId}/content`);
  expect(reopened.querySelector("pre")).toHaveAttribute("data-language", "javascript");
  expect(within(reopened).getByText("保留的第二段")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("combobox", { name: "代码语言" }));
  const python = await screen.findByRole("option", { name: "Python" }); fireEvent.pointerDown(python, { pointerType: "mouse" }); fireEvent.click(python);
  fireEvent.click(screen.getByRole("button", { name: "保存并记录历史" }));
  await waitFor(() => expect(calls("PATCH", id(1))).toHaveLength(2));
  const afterReturn = await calls("PATCH", id(1))[1]!.json();
  expect(afterReturn.expected_version).toBe(2);
  expect(afterReturn.document.blocks).toHaveLength(3);
  expect(afterReturn.document.blocks).toEqual(expect.arrayContaining([
    expect.objectContaining({ id: "code-return", type: "codeBlock", language: "python", content: [{ type: "text", text: "retained code();" }] }),
    expect.objectContaining({ type: "image", asset_id: assetId }),
    expect.objectContaining({ id: "paragraph-return", type: "paragraph", content: [{ type: "text", text: "保留的第二段" }] }),
  ]));
});

it("uppercase UUID retains the initialized draft after401 and a failed reauthentication, then retries the original CAS", async () => {
  const canonical = "abcdefab-cdef-4abc-8def-abcdefabcdef";
  const upper = canonical.toUpperCase(); expect(upper).not.toBe(canonical);
  const path = `/notes/${upper}`; expect(safeReturnTo(path)).toBe(path);
  notes[0] = { ...note(1), id: canonical, title: "大写路径笔记", document: doc("认证失败仍保留的正文") };
  let expired = false;
  intercept = (r, u) => {
    if (r.method === "GET" && u.pathname.toLowerCase() === `/api/v1/notes/${canonical}`) return expired ? failure(401) : json(notes[0]);
    if (expired && u.pathname.endsWith("/auth/session")) return json({ code: "SESSION_EXPIRED", message: "重新认证仍然失效" }, 401);
    if (expired && (r.method === "PATCH" && u.pathname.endsWith(canonical) || r.method === "GET" && u.pathname === "/api/v1/notes")) return failure(401);
    return undefined;
  };
  const { bundle, router } = await mount(path); const input = await title();
  const editor = screen.getByRole("region", { name: "当前笔记" }).querySelector('[contenteditable="true"]');
  expired = true; fireEvent.change(input, { target: { value: "大写路径失效后保留草稿" } });
  await waitFor(() => expect(calls("PATCH", canonical)).toHaveLength(1), { timeout: 2500 });
  await screen.findByText("会话已失效，本地草稿已保留");
  await waitFor(() => expect(bundle.queryClient.getQueryData(queryKeys.auth.session())).toBeUndefined());
  expect(screen.getByRole("textbox", { name: "笔记标题" })).toHaveValue("大写路径失效后保留草稿");
  fireEvent.click(screen.getByRole("button", { name: "已重新登录，重试原版本" }));
  await waitFor(() => expect(calls("GET", "/auth/session")).toHaveLength(2));
  await screen.findByText("重新认证仍然失效");
  expect(screen.getByRole("textbox", { name: "笔记标题" })).toHaveValue("大写路径失效后保留草稿");
  expect(screen.getByText("认证失败仍保留的正文")).toBeInTheDocument();
  expect(screen.getByRole("region", { name: "当前笔记" }).querySelector('[contenteditable="false"]')).toBe(editor);
  expect(screen.getByText(/当前基础版本 1/)).toBeInTheDocument(); expect(router.state.location.pathname).toBe(path);
  expect(calls("PATCH", canonical)).toHaveLength(1);
  await selectNoteAction("复制当前草稿");
  expect(navigator.clipboard.writeText).toHaveBeenCalledWith(expect.stringContaining("大写路径失效后保留草稿"));
  expect(navigator.clipboard.writeText).toHaveBeenCalledWith(expect.stringContaining("认证失败仍保留的正文"));
  expired = false; fireEvent.click(screen.getByRole("button", { name: "已重新登录，重试原版本" }));
  await waitFor(() => expect(calls("PATCH", canonical)).toHaveLength(2));
  expect(await calls("PATCH", canonical)[1]!.json()).toEqual({ expected_version: 1, title: "大写路径失效后保留草稿", document: doc("认证失败仍保留的正文") });
  await waitFor(() => expect(screen.getByRole("textbox", { name: "笔记标题" })).toBeEnabled());
  expect(screen.getByRole("textbox", { name: "笔记标题" })).toHaveValue("大写路径失效后保留草稿");
  expect(router.state.location.pathname).toBe(path);
});

it("t67_editor refuses to insert a reclaimed image returned on upload replay", async () => {
  intercept = (request, url) => request.method === "POST" && url.pathname === "/api/v1/assets"
    ? json({ id: id(98), mime: "image/png", size: 68, sha256: "a".repeat(64), state: "deleted", created_at: "2026-11-17T00:00:00Z" }, 201) : undefined;
  await mount();
  await title();
  const form = await new BrowserRequest("/", { method: "POST", headers: { "Content-Type": "application/x-www-form-urlencoded" }, body: "" }).formData();
  vi.stubGlobal("FormData", form.constructor);
  vi.stubGlobal("File", NodeFile);
  fireEvent.change(screen.getByLabelText("选择文件"), {
    target: { files: [new File(["public PNG fixture"], "retired.png", { type: "image/png" })] },
  });
  expect(await screen.findByText("图片已回收，请重新选择文件")).toBeInTheDocument();
  expect(screen.queryByRole("img", { name: "笔记图片" })).not.toBeInTheDocument();
  expect(calls("PATCH", id(1))).toHaveLength(0);
});

it("move dialog uses TreeSelect to pick a non-root parent while excluding the current note", async () => {
  await mount();
  await title();
  await selectNoteAction("移动笔记");
  const dialog = await screen.findByRole("dialog", { name: "移动笔记" });
  fireEvent.click(within(dialog).getByRole("combobox", { name: "目标父笔记" }));
  const tree = await screen.findByRole("tree", { name: "目标父笔记" });
  expect(within(tree).queryByRole("treeitem", { name: "笔记1" })).not.toBeInTheDocument();
  fireEvent.click(within(tree).getByRole("treeitem", { name: "笔记2" }));
  fireEvent.click(within(dialog).getByRole("button", { name: "确认移动" }));
  await waitFor(() => expect(calls("POST", "/move")).toHaveLength(1));
  expect(await calls("POST", "/move")[0]!.json()).toEqual({ expected_version: 1, parent_id: id(2), position: 0 });
});
