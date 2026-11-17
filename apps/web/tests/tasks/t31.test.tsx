import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { createBrowserHistory, createMemoryHistory } from "@tanstack/react-router";
import { afterAll, afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import type { components } from "@knoted/api-client";
import App from "../../src/App";
import { createAppRouter, safeReturnTo } from "../../src/lib/router";
import { queryKeys } from "../../src/lib/query";
import { UiProvider } from "../../src/lib/ui-provider";

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
  fireEvent.click(screen.getByRole("button", { name: "移动笔记" }));
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
  fireEvent.click(screen.getByRole("button", { name: "移动笔记" })); fireEvent.click(await screen.findByRole("button", { name: "确认移动" }));
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
  const { router } = await mount(); await title(); fireEvent.click(screen.getByRole("button", { name: "删除笔记" }));
  expect(await screen.findByText(/仍有子页/)).toBeInTheDocument(); fireEvent.click(screen.getByRole("button", { name: "确认删除" }));
  await waitFor(() => expect(calls("DELETE", id(1))).toHaveLength(1)); expect(router.state.location.pathname).toBe(`/notes/${id(1)}`);
  fireEvent.click(screen.getByRole("button", { name: "取消" }));
  await act(async () => { await router.navigate({ to: "/notes/$noteId", params: { noteId: id(2) } }); });
  fireEvent.click(await screen.findByRole("button", { name: "删除笔记" })); fireEvent.click(await screen.findByRole("button", { name: "确认删除" }));
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
  fireEvent.click(screen.getByRole("button", { name: "复制当前草稿" })); expect(navigator.clipboard.writeText).toHaveBeenCalledWith(expect.stringContaining("过期也保留"));
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
  fireEvent.click(screen.getByRole("button", { name: "移动笔记" })); fireEvent.click(await screen.findByRole("button", { name: "确认移动" }));
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
  expect(screen.getByRole("region", { name: "笔记标签与资料关联" })).toBeInTheDocument();
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
  await mount(); const input = await title(); fireEvent.click(screen.getByRole("button", { name: "移动笔记" })); fireEvent.click(await screen.findByRole("button", { name: "确认移动" }));
  await waitFor(() => expect(calls("POST", "/snapshots")).toHaveLength(1)); await act(async () => links.resolve(failure(401)));
  await screen.findByText("会话已失效，本地草稿已保留"); await act(async () => history.resolve(json(snapshot(notes[0]!), 201)));
  await waitFor(() => expect(screen.getAllByText(/会话已失效，草稿已保留/).length).toBeGreaterThan(0));
  expect(calls("POST", "/move")).toHaveLength(0); expect(input).toHaveValue("笔记1");
});
it("association401 during pending history prevents a subsequently queued DELETE", async () => {
  const links = deferred(); const history = deferred(); intercept = (r, u) => u.pathname.endsWith("/links") ? links.promise : r.method === "POST" && u.pathname.endsWith("/snapshots") ? history.promise : undefined;
  const { router } = await mount(); const input = await title(); fireEvent.click(screen.getByRole("button", { name: "删除笔记" })); fireEvent.click(await screen.findByRole("button", { name: "确认删除" }));
  await waitFor(() => expect(calls("POST", "/snapshots")).toHaveLength(1)); await act(async () => links.resolve(failure(401)));
  await screen.findByText("会话已失效，本地草稿已保留"); await act(async () => history.resolve(json(snapshot(notes[0]!), 201)));
  await waitFor(() => expect(screen.getByRole("button", { name: "确认删除" })).toBeEnabled());
  expect(calls("DELETE", id(1))).toHaveLength(0); expect(router.state.location.pathname).toBe(`/notes/${id(1)}`); expect(input).toHaveValue("笔记1");
});

it("removing a child allows its parent DELETE, and each confirmed deletion leaves an independent receipt", async () => {
  notes = [note(1), note(2, 1)]; intercept = (r, u) => r.method === "DELETE" && notes.some(n => n.parent_id === u.pathname.split("/").at(-1)) ? failure(409) : undefined;
  const { router } = await mount(); await title(); fireEvent.click(screen.getByRole("button", { name: "删除笔记" })); fireEvent.click(await screen.findByRole("button", { name: "确认删除" }));
  await waitFor(() => expect(calls("DELETE", id(1))).toHaveLength(1)); expect(notes).toHaveLength(2);
  fireEvent.click(screen.getByRole("button", { name: "取消" }));
  await act(async () => { await router.navigate({ to: "/notes/$noteId", params: { noteId: id(2) } }); });
  fireEvent.click(await screen.findByRole("button", { name: "删除笔记" })); fireEvent.click(await screen.findByRole("button", { name: "确认删除" }));
  await waitFor(() => expect(router.state.location.pathname).toBe("/notes")); expect(notes.map(n => n.id)).toEqual([id(1)]);
  fireEvent.click(await screen.findByRole("treeitem", { name: "笔记1" }));
  fireEvent.click(await screen.findByRole("button", { name: "删除笔记" })); fireEvent.click(await screen.findByRole("button", { name: "确认删除" }));
  await waitFor(() => expect(router.state.location.pathname).toBe("/notes")); expect(notes).toHaveLength(0);
  expect(screen.getByRole("button", { name: "恢复本次删除 笔记1" })).toBeInTheDocument(); expect(screen.getByRole("button", { name: "恢复本次删除 笔记2" })).toBeInTheDocument();
});
it("confirmed title saves update tree labels without projecting an unsaved draft into the tree", async () => {
  await mount(); const input = await title(); fireEvent.change(input, { target: { value: "树上的已确认标题" } });
  expect(screen.getByRole("treeitem", { name: "笔记1" })).toBeInTheDocument(); expect(screen.queryByRole("treeitem", { name: "树上的已确认标题" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "保存并记录历史" }));
  expect(await screen.findByRole("treeitem", { name: "树上的已确认标题" })).toBeInTheDocument();
});
