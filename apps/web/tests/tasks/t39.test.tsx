import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createMemoryHistory } from "@tanstack/react-router";
import App from "../../src/App";
import { createAppRouter, safeReturnTo } from "../../src/lib/router";
import { createApiClient, type components } from "@knoted/api-client";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { dateBoundary } from "../../src/features/tasks/dates";
import { TasksPage } from "../../src/features/tasks/TasksPage";
import { UiProvider } from "../../src/lib/ui-provider";
import { queryKeys, type QueryClientBundle } from "../../src/lib/query";

type Task = components["schemas"]["Task"];
const stamp = "2026-11-17T00:00:00Z";
const a = "00000000-0000-4000-8000-000000000001";
const b = "00000000-0000-4000-8000-000000000002";
const p = "00000000-0000-4000-8000-000000000003";
const q = "00000000-0000-4000-8000-000000000004";
function task(id: string, title: string, patch: Partial<Task> = {}): Task {
  return { id, title, project_id: null, parent_id: null, description: null, status: "todo", priority: "none", start: null, due: null, version: 1, archived_at: null, deleted_at: null, created_at: stamp, updated_at: stamp, ...patch };
}
const json = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status, headers: { "content-type": "application/json" } });
function harness(rows = [task(a, "父事项"), task(b, "第二事项")], cursorMode = false, failSecondPage = false) {
  const requests: Request[] = [];
  let rejectField = false;
  let failRefresh = false;
  const projects: components["schemas"]["Project"][] = [p, q].map((id, i) => ({ id, title: `项目${i + 1}`, description: null, version: 1, archived_at: null, deleted_at: null, created_at: stamp, updated_at: stamp }));
  const fetch: typeof globalThis.fetch = async (input) => {
    const req = input as Request;
    requests.push(req.clone());
    const url = new URL(req.url);
    const path = url.pathname.replace("/api/v1", "");
    const error = (status: number, extra = {}) => json({ code: status === 409 ? "version_conflict" : "validation_failed", message: "请求失败", request_id: a, ...extra }, status);
    if (path === "/auth/session") return json({ session: { id: a, user_id: b, current: true, created_at: stamp, last_seen_at: stamp, idle_expires_at: stamp, absolute_expires_at: stamp }, csrf_token: "test-csrf-value-long" });
    if (path === "/projects") return json({ items: projects, next_cursor: null });
    if (path.startsWith("/projects/")) return json(projects.find((v) => v.id === path.split("/")[2]));
    if (path === "/tasks" && req.method === "GET") {
      const items = rows.filter((r) => !r.deleted_at && (!url.searchParams.has("archived") || String(!!r.archived_at) === url.searchParams.get("archived")) && (!url.searchParams.has("parent_id") || r.parent_id === url.searchParams.get("parent_id")) && (!url.searchParams.has("project_id") || r.project_id === url.searchParams.get("project_id")));
      if (cursorMode && url.searchParams.has("project_id")) {
        const next = url.searchParams.get("cursor") === "tasks-second";
        if (next && failSecondPage) return error(503);
        return json({ items: items.slice(next ? 1 : 0, next ? 2 : 1), next_cursor: next ? null : "tasks-second" });
      }
      return json({ items, next_cursor: null });
    }
    if (path === "/tasks" && req.method === "POST") {
      const body = await req.json() as components["schemas"]["TaskCreate"];
      const parent = rows.find((r) => r.id === body.parent_id);
      const row = task(`00000000-0000-4000-8000-00000000000${rows.length + 1}`, body.title, { ...body, project_id: parent ? parent.project_id : body.project_id ?? null, parent_id: body.parent_id ?? null });
      rows.push(row);
      return json(row, 201);
    }
    const row = rows.find((r) => r.id === path.split("/")[2]);
    if (!row) return error(404);
    if (req.method === "GET") return failRefresh ? error(503) : json(row);
    const body = req.method === "DELETE" ? { expected_version: Number(url.searchParams.get("expected_version")) } : await req.json() as components["schemas"]["TaskUpdate"];
    if (body.expected_version !== row.version) return error(409, { current_version: row.version });
    if (rejectField) return error(422, { field_errors: [{ field: "start/due", message: "日期无效" }] });
    if (req.method === "DELETE") { row.deleted_at = stamp; return new Response(null, { status: 204 }); }
    if (path.endsWith("/archive")) row.archived_at = stamp;
    else if (path.endsWith("/unarchive")) row.archived_at = null;
    else { Object.assign(row, body); for (const child of rows.filter((v) => v.parent_id === row.id)) child.project_id = row.project_id; }
    row.version += 1;
    return json(row);
  };
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const bundle = { queryClient, api: createApiClient({ baseUrl: "http://localhost/api/v1", fetch }) } satisfies QueryClientBundle;
  return { rows, projects, requests, bundle, reject: () => { rejectField = true; }, failRefresh: () => { failRefresh = true; } };
}
function Page({ bundle }: { bundle: QueryClientBundle }) {
  const [id, setId] = useState<string>();
  return <>{id && <button onClick={() => setId(undefined)}>返回事项</button>}
    <TasksPage bundle={bundle} taskId={id} onSelectTask={(next) => setId(next ?? undefined)} />
  </>;
}
function renderPage(bundle: QueryClientBundle) {
  return render(<UiProvider><QueryClientProvider client={bundle.queryClient}><Page bundle={bundle} /></QueryClientProvider></UiProvider>);
}
async function open(title = "父事项") { fireEvent.click(await screen.findByRole("link", { name: title })); return screen.findByRole("region", { name: "事项详情" }); }
async function openEdit() {
  const trigger = await screen.findByRole("button", { name: "编辑事项" });
  await waitFor(() => expect(trigger).toBeEnabled());
  fireEvent.click(trigger);
  return within(await screen.findByRole("dialog", { name: "编辑事项" }));
}
async function taskAction(name: string) {
  const trigger = await screen.findByRole("button", { name: "更多事项操作" });
  await waitFor(() => expect(trigger).toBeEnabled());
  fireEvent.click(trigger);
  fireEvent.click(await screen.findByRole("menuitem", { name }));
}
async function createTaskDialog(name = "新建事项", save = "创建事项") {
  fireEvent.click(await screen.findByRole("button", { name }));
  const dialog = within(await screen.findByRole("dialog", { name }));
  await waitFor(() => expect(dialog.getByRole("button", { name: save })).toBeEnabled());
  return dialog;
}
async function select(label: string, option: string, scope: Pick<typeof screen, "getByRole"> = screen) {
  fireEvent.click(scope.getByRole("combobox", { name: label }));
  const item = await screen.findByRole("option", { name: option });
  fireEvent.pointerDown(item, { pointerType: "mouse" }); fireEvent.click(item);
}

describe("T39 事项页面", () => {
  it.each([
    ["2026-11-17", null, false, "2026-11-16T16:00:00.000Z"],
    ["2026-11-17", null, true, "2026-11-17T16:00:00.000Z"],
    ["1991-07-01", "12:00", true, "1991-07-01T03:00:00.000Z"],
    ["2026-02-30", null, false, null],
    ["2026-11-17", "24:00", false, null],
    ["2026-11-17", "9:30", false, null],
  ] as const)("上海 DateValue %s %s due=%s", (date, time, due, expected) => {
    const instant = dateBoundary({ date, time, timezone: "Asia/Shanghai" }, due);
    expect(instant === null ? null : new Date(instant).toISOString()).toBe(expected);
  });
  it("首次读取清单时不显示成功空态", async () => {
    const h = harness();
    renderPage(h.bundle);
    expect(screen.getByLabelText("正在读取事项")).toBeInTheDocument();
    expect(screen.queryByText("暂无事项")).not.toBeInTheDocument();
  });

  it("事项深链可直接访问且登录保留安全站内目的地", async () => {
    const h = harness();
    expect(safeReturnTo(`/tasks/${a}`)).toBe(`/tasks/${a}`);
    expect(safeReturnTo("/tasks")).toBe("/tasks");
    for (const value of ["//evil/tasks", "/tasks/no-id", `/tasks/${a}?redirect=//evil`, `/tasks/${a}/other`]) expect(safeReturnTo(value)).toBe("/");
    const router = createAppRouter({ history: createMemoryHistory({ initialEntries: [`/tasks/${a}`] }), bundle: h.bundle });
    render(<UiProvider><App router={router} /></UiProvider>);
    await act(async () => { await router.load(); });
    expect(await screen.findByRole("heading", { level: 1, name: "父事项" })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "事项列表" })).not.toBeInTheDocument();
    expect(screen.queryByLabelText("事项标题")).not.toBeInTheDocument();
  });
  it("事项第二页读取失败时仍可返回上一页", async () => {
    const h = harness([task(a, "父事项", { project_id: p }), task(b, "第二事项", { project_id: p })], true, true);
    renderPage(h.bundle);
    await waitFor(() => expect(screen.getByRole("combobox", { name: "项目筛选" })).toBeEnabled());
    await select("项目筛选", "项目1");
    const next = await screen.findByRole("button", { name: "下一页事项" });
    await waitFor(() => expect(next).toBeEnabled());
    fireEvent.click(next);
    await screen.findByRole("button", { name: "重试事项列表" });
    fireEvent.click(screen.getByRole("button", { name: "上一页事项" }));
    expect(await screen.findByRole("link", { name: "父事项" })).toBeInTheDocument();
  });

  it("从筛选后的看板第二页打开事项并返回原看板", async () => {
    const h = harness([task(a, "父事项", { project_id: p }), task(b, "第二事项", { project_id: p })], true);
    const router = createAppRouter({ history: createMemoryHistory({ initialEntries: ["/tasks"] }), bundle: h.bundle });
    render(<UiProvider><App router={router} /></UiProvider>);
    await act(async () => { await router.load(); });
    screen.getByRole("main").scrollTo = () => {};
    await waitFor(() => expect(screen.getByRole("combobox", { name: "项目筛选" })).toBeEnabled());
    await select("项目筛选", "项目1");
    fireEvent.click(screen.getByRole("radio", { name: "看板视图" }));
    const next = await screen.findByRole("button", { name: "下一页事项" });
    await waitFor(() => expect(next).toBeEnabled());
    fireEvent.click(next);
    await open("第二事项");
    expect(screen.getByRole("heading", { level: 1, name: "第二事项" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("link", { name: "返回事项" }));
    expect(await screen.findByRole("radio", { name: "看板视图" })).toBeChecked();
    expect(screen.getByRole("combobox", { name: "项目筛选" })).toHaveTextContent("项目1");
    expect(await screen.findByRole("link", { name: "第二事项" })).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "父事项" })).not.toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("button", { name: "上一页事项" })).toBeEnabled());
  });

  it("t39_1 阻止上海不存在及重复的时间和倒置区间，保留纯日期与精确时间", async () => {
    const h = harness(); renderPage(h.bundle); await open(); const detail = await openEdit();
    fireEvent.change(detail.getByLabelText("开始日期"), { target: { value: "1991-04-14" } });
    fireEvent.change(detail.getByLabelText("开始时间"), { target: { value: "02:30" } });
    fireEvent.click(detail.getByRole("button", { name: "保存事项" }));
    expect(await detail.findByText(/不存在或重复/)).toBeInTheDocument();
    expect(h.requests.filter((r) => r.method === "PATCH")).toHaveLength(0);
    fireEvent.change(detail.getByLabelText("开始日期"), { target: { value: "1991-09-15" } });
    fireEvent.change(detail.getByLabelText("开始时间"), { target: { value: "01:30" } });
    fireEvent.click(detail.getByRole("button", { name: "保存事项" }));
    expect(await detail.findByText(/不存在或重复/)).toBeInTheDocument();
    fireEvent.change(detail.getByLabelText("开始日期"), { target: { value: "2026-11-18" } });
    fireEvent.change(detail.getByLabelText("截止日期"), { target: { value: "2026-11-17" } });
    fireEvent.click(detail.getByRole("button", { name: "保存事项" }));
    expect(await detail.findByText(/开始不能晚于截止/)).toBeInTheDocument();
    fireEvent.change(detail.getByLabelText("开始日期"), { target: { value: "2026-11-17" } });
    fireEvent.change(detail.getByLabelText("开始时间"), { target: { value: "10:30" } });
    fireEvent.click(detail.getByRole("button", { name: "保存事项" }));
    await waitFor(() => expect(h.rows[0]!.start).toEqual({ date: "2026-11-17", time: "10:30", timezone: "Asia/Shanghai" }));
    expect(h.rows[0]!.due).toEqual({ date: "2026-11-17", time: null, timezone: "Asia/Shanghai" });
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    const serverValidation = await openEdit();
    h.reject();
    fireEvent.change(serverValidation.getByLabelText("事项标题"), { target: { value: "保留" } });
    fireEvent.click(serverValidation.getByRole("button", { name: "保存事项" }));
    expect(await serverValidation.findByText("日期无效")).toBeInTheDocument();
    expect(serverValidation.getByLabelText("开始日期")).toHaveAttribute("aria-invalid", "true");
  });
  it("t39_2 父事项换项目后刷新子列表和子详情的继承项目", async () => {
    const h = harness([task(a, "父事项", { project_id: p }), task(b, "子事项", { parent_id: a, project_id: p })]);
    renderPage(h.bundle); await open();
    expect(await within(await screen.findByRole("region", { name: "子事项" })).findByText("项目1")).toBeInTheDocument();
    const detail = await openEdit();
    await select("所属项目", "项目2", detail);
    fireEvent.click(detail.getByRole("button", { name: "保存事项" }));
    await waitFor(() => expect(h.rows[1]!.project_id).toBe(q));
    expect(await within(screen.getByRole("list", { name: "子事项列表" })).findByText("项目2")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    fireEvent.click(within(screen.getByRole("region", { name: "子事项" })).getByRole("link", { name: "子事项" }));
    expect(await screen.findByRole("heading", { name: "子事项", level: 1 })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "子事项" })).not.toBeInTheDocument();
    const childEdit = await openEdit();
    expect(childEdit.getByLabelText("事项标题")).toHaveValue("子事项");
    expect(childEdit.getByRole("combobox", { name: "所属项目" })).toBeDisabled();
  });
  it("t39_3 409保留可复制草稿，刷新版本才能重试，切换事项不串草稿", async () => {
    const h = harness(); const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    renderPage(h.bundle); await open(); const detail = await openEdit();
    h.rows[0]!.version = 7;
    fireEvent.change(detail.getByLabelText("事项标题"), { target: { value: "我的草稿" } });
    fireEvent.click(detail.getByRole("button", { name: "保存事项" }));
    expect(await detail.findByText(/草稿已保留/)).toHaveTextContent("7");
    expect(detail.getByLabelText("事项标题")).toHaveValue("我的草稿");
    fireEvent.click(detail.getByRole("button", { name: "复制草稿" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(expect.stringContaining("我的草稿")));
    expect(detail.getByRole("button", { name: "保存事项" })).toBeDisabled();
    fireEvent.click(detail.getByRole("button", { name: "刷新服务端版本" }));
    await waitFor(() => expect(detail.getByRole("button", { name: "保存事项" })).toBeEnabled());
    fireEvent.click(detail.getByRole("button", { name: "保存事项" }));
    await waitFor(() => expect(h.rows[0]!.version).toBe(8));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: "返回事项" }));
    await open("第二事项");
    expect((await openEdit()).getByLabelText("事项标题")).toHaveValue("第二事项");
  });
  it("后台刷新后仍以草稿原版本保存，不能覆盖远端说明和日期", async () => {
    const h = harness(); renderPage(h.bundle); await open(); const detail = await openEdit();
    fireEvent.change(detail.getByLabelText("事项标题"), { target: { value: "本地标题草稿" } });
    h.rows[0]!.description = "远端说明";
    h.rows[0]!.due = { date: "2026-12-20", time: "10:30", timezone: "Asia/Shanghai" };
    h.rows[0]!.version = 2;
    await act(async () => {
      await h.bundle.queryClient.invalidateQueries({ queryKey: queryKeys.tasks.all() });
    });
    await waitFor(() => expect(h.bundle.queryClient.getQueryData<Task>(queryKeys.tasks.detail(a))?.version).toBe(2));
    expect(detail.getByLabelText("事项标题")).toHaveValue("本地标题草稿");
    fireEvent.click(detail.getByRole("button", { name: "保存事项" }));
    expect(await detail.findByText(/草稿已保留/)).toBeInTheDocument();
    expect(h.rows[0]!.description).toBe("远端说明");
    expect(h.rows[0]!.due).toEqual({ date: "2026-12-20", time: "10:30", timezone: "Asia/Shanghai" });
    const patch = h.requests.find((request) => request.method === "PATCH");
    expect(await patch?.json()).toMatchObject({ expected_version: 1, title: "本地标题草稿" });
  });
  it("未修改表单随普通刷新同步，成功保存后使用新的基础版本", async () => {
    const h = harness(); renderPage(h.bundle); await open(); const detail = await openEdit();
    h.rows[0]!.title = "远端标题"; h.rows[0]!.description = "远端说明"; h.rows[0]!.version = 2;
    await act(async () => { await h.bundle.queryClient.invalidateQueries({ queryKey: queryKeys.tasks.all() }); });
    await waitFor(() => expect(detail.getByLabelText("事项标题")).toHaveValue("远端标题"));
    expect(detail.getByLabelText("事项说明")).toHaveValue("远端说明");
    fireEvent.change(detail.getByLabelText("事项标题"), { target: { value: "第一次保存" } });
    fireEvent.click(detail.getByRole("button", { name: "保存事项" }));
    await waitFor(() => expect(h.rows[0]!.version).toBe(3));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    const again = await openEdit();
    fireEvent.change(again.getByLabelText("事项标题"), { target: { value: "再次保存" } });
    fireEvent.click(again.getByRole("button", { name: "保存事项" }));
    await waitFor(() => expect(h.rows[0]!.version).toBe(4));
    expect(h.rows[0]!.description).toBe("远端说明");
  });
  it("刷新失败继续禁止冲突重试，草稿仍可手动复制", async () => {
    const h = harness(); renderPage(h.bundle); await open(); const detail = await openEdit();
    h.rows[0]!.version = 2; h.failRefresh();
    fireEvent.change(detail.getByLabelText("事项标题"), { target: { value: "暂存草稿" } });
    fireEvent.click(detail.getByRole("button", { name: "保存事项" }));
    await detail.findByText(/草稿已保留/);
    fireEvent.click(detail.getByRole("button", { name: "刷新服务端版本" }));
    expect(await detail.findByText(/刷新失败/)).toBeInTheDocument();
    expect(detail.getByRole("button", { name: "保存事项" })).toBeDisabled();
    expect((detail.getByLabelText("草稿副本") as HTMLTextAreaElement).value).toContain("暂存草稿");
  });
  it("创建独立事项、项目事项和单层子事项；删除确认取消不发请求", async () => {
    const h = harness(); renderPage(h.bundle);
    let create = await createTaskDialog();
    fireEvent.change(create.getByLabelText("事项标题"), { target: { value: "新独立事项" } });
    fireEvent.click(create.getByRole("button", { name: "创建事项" }));
    await waitFor(() => expect(h.rows.some((r) => r.title === "新独立事项" && r.project_id === null)).toBe(true));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    create = await createTaskDialog();
    expect(create.getByLabelText("事项标题")).toHaveValue("");
    fireEvent.change(create.getByLabelText("事项标题"), { target: { value: "新项目事项" } });
    await select("所属项目", "项目1", create);
    fireEvent.click(create.getByRole("button", { name: "创建事项" }));
    await waitFor(() => expect(h.rows.some((r) => r.title === "新项目事项" && r.project_id === p)).toBe(true));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    await open();
    const child = await createTaskDialog("新建子事项", "创建子事项");
    fireEvent.change(child.getByLabelText("事项标题"), { target: { value: "新子事项" } });
    fireEvent.click(child.getByRole("button", { name: "创建子事项" }));
    expect(await within(await screen.findByRole("region", { name: "子事项" })).findByText("新子事项")).toBeInTheDocument();
    expect(h.rows.find((r) => r.title === "新子事项")?.parent_id).toBe(a);
    await taskAction("删除事项");
    fireEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "取消" }));
    expect(h.requests.some((r) => r.method === "DELETE")).toBe(false);
  });
  it("归档和取消归档后刷新读写状态，删除成功关闭详情", async () => {
    const h = harness(); renderPage(h.bundle); const detail = within(await open());
    await taskAction("归档事项");
    await waitFor(() => expect(detail.getByRole("button", { name: "编辑事项" })).toBeDisabled());
    fireEvent.click(await detail.findByRole("button", { name: "取消事项归档" }));
    await waitFor(() => expect(detail.getByRole("button", { name: "编辑事项" })).toBeEnabled());
    await taskAction("删除事项");
    fireEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "确认删除事项" }));
    await waitFor(() => expect(screen.queryByRole("region", { name: "事项详情" })).not.toBeInTheDocument());
    expect(h.rows[0]!.deleted_at).toBe(stamp);
  });
  it("保存显式清空日期和父关系", async () => {
    const h = harness([task(a, "父事项"), task(b, "子事项", { parent_id: a, start: { date: "2026-11-17", time: "09:30", timezone: "Asia/Shanghai" } })]);
    renderPage(h.bundle); await open("子事项"); const detail = await openEdit();
    await waitFor(() => expect(detail.getByRole("button", { name: "保存事项" })).toBeEnabled());
    fireEvent.change(detail.getByLabelText("开始日期"), { target: { value: "" } });
    fireEvent.change(detail.getByLabelText("开始时间"), { target: { value: "" } });
    await select("父事项", "顶层事项", detail);
    fireEvent.click(detail.getByRole("button", { name: "保存事项" }));
    await waitFor(() => expect(h.rows[1]!.start).toBeNull());
    expect(h.rows[1]!.parent_id).toBeNull();
  });
  it("父或项目归档只读，取消自身归档不绕过祖先写态", async () => {
    const h = harness([task(a, "父事项", { archived_at: stamp }), task(b, "子事项", { parent_id: a })]);
    renderPage(h.bundle); const detail = within(await open("子事项"));
    expect(await detail.findByText(/父事项或项目不可写/)).toBeInTheDocument();
    expect(detail.getByRole("button", { name: "编辑事项" })).toBeDisabled();
    expect(detail.getByRole("button", { name: "更多事项操作" })).toBeDisabled();
    await act(async () => { h.projects[0]!.archived_at = stamp; h.rows[1]!.parent_id = null; h.rows[1]!.project_id = p; await h.bundle.queryClient.invalidateQueries({ queryKey: queryKeys.tasks.all() }); });
    await waitFor(() => expect(detail.getByRole("button", { name: "编辑事项" })).toBeDisabled());
  });
});
