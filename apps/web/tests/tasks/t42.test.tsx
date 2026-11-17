import { act, fireEvent, render, screen, within, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createMemoryHistory } from "@tanstack/react-router";
import App from "../../src/App";
import { createAppRouter, safeReturnTo } from "../../src/lib/router";
import { createApiClient, type components } from "@knoted/api-client";
import { expect, it, vi } from "vitest";
import { TrashPage } from "../../src/features/trash/TrashPage";
import { UiProvider } from "../../src/lib/ui-provider";
it("回收站通过真实受保护路由可访问，侧栏启用且允许登录返回", async () => {
    const scroll = vi.spyOn(window, "scrollTo").mockImplementation(() => { });
    const bundle = { queryClient: new QueryClient({ defaultOptions: { queries: { retry: false } } }), api: createApiClient({ baseUrl: "http://localhost/api/v1", fetch: async (input) => new Response(JSON.stringify((input as Request).url.endsWith("/auth/session") ? { csrf_token: "test-csrf", session: { id: "1" } } : { items: [], next_cursor: null }), { headers: { "content-type": "application/json" } }) }) };
    const router = createAppRouter({ bundle, history: createMemoryHistory({ initialEntries: ["/trash"] }) });
    render(<UiProvider><QueryClientProvider client={bundle.queryClient}><App router={router}/></QueryClientProvider></UiProvider>);
    expect(await screen.findByRole("heading", { name: "回收站" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "回收站" })).toBeEnabled();
    expect(safeReturnTo("/trash")).toBe("/trash");
    scroll.mockRestore();
});
type Item = components["schemas"]["TrashItem"];
const item = (id: string, title: string, patch: Partial<Item> = {}): Item => ({ id, title, entity_type: "note", deleted_at: "2026-10-01T00:00:00Z", parent_id: null, version: 7, can_restore: true, reason: null, ...patch });
function harness(rows: Item[]) {
    let failRead = false;
    let failWrite = false;
    let pause = false;
    let release: () => void = () => { };
    const requests: Request[] = [];
    const bundle = { queryClient: new QueryClient({ defaultOptions: { queries: { retry: false } } }), api: createApiClient({ baseUrl: "http://localhost/api/v1", fetch: async (input) => {
                const r = input as Request;
                requests.push(r.clone());
                const url = new URL(r.url);
                const json = (v: unknown, status = 200) => new Response(JSON.stringify(v), { status, headers: { "content-type": "application/json" } });
                if (r.method === "GET") {
                    if (pause)
                        await new Promise<void>(resolve => { release = resolve; });
                    if (failRead)
                        return json({ code: "dependency_unavailable", message: "SQL private", request_id: "x" }, 503);
                    const filter = url.searchParams.get("entity_type");
                    return json({ items: rows.filter(x => !filter || x.entity_type === filter), next_cursor: null });
                }
                if (failWrite)
                    return json({ code: "version_conflict", message: "private detail", request_id: "x" }, 409);
                const id = url.pathname.split("/").at(-2);
                rows = rows.filter(x => x.id !== id);
                return json({ id });
            } }) };
    const mount = () => render(<UiProvider><QueryClientProvider client={bundle.queryClient}><TrashPage bundle={bundle}/></QueryClientProvider></UiProvider>);
    return { bundle, mount, requests, failRead: () => { failRead = true; }, failWrite: () => { failWrite = true; }, recoverWrite: () => { failWrite = false; }, pause: () => { pause = true; }, release: () => { pause = false; release(); } };
}
it("t42_1 父缺失提示允许显式恢复根目录，确认/取消后才提交精确CAS", async () => {
    const h = harness([item("a", "子笔记", { can_restore: false, reason: "父笔记缺失，请先恢复父笔记或恢复到根目录。" })]);
    h.mount();
    const row = await screen.findByRole("region", { name: "回收对象 子笔记" });
    expect(row).toHaveTextContent("父笔记缺失");
    expect(within(row).getByRole("button", { name: "恢复 子笔记" })).toBeDisabled();
    fireEvent.click(within(row).getByRole("combobox", { name: "恢复位置 子笔记" }));
    {
        const option = await screen.findByRole("option", { name: "根目录" });
        fireEvent.pointerDown(option, { pointerType: "mouse" });
        fireEvent.click(option);
    }
    fireEvent.click(within(row).getByRole("button", { name: "恢复 子笔记" }));
    fireEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(h.requests.filter(r => r.method === "POST")).toHaveLength(0);
    fireEvent.click(within(row).getByRole("button", { name: "恢复 子笔记" }));
    fireEvent.click(screen.getByRole("button", { name: "确认恢复" }));
    await screen.findByText("已恢复 子笔记");
    const write = h.requests.find(r => r.method === "POST")!;
    expect(write.url).toBe("http://localhost/api/v1/notes/a/restore");
    expect(await write.json()).toEqual({ expected_version: 7, restore_parent: { parent_id: null } });
    expect(screen.getByText(/30 天/)).toBeInTheDocument();
});
it("t42_3 其他对象恢复并刷新时保留本行位置选择、冲突反馈", async () => {
    const h = harness([item("a", "原对象"), item("b", "另一个", { entity_type: "project" })]);
    h.mount();
    const row = await screen.findByRole("region", { name: "回收对象 原对象" });
    fireEvent.click(within(row).getByRole("combobox", { name: "恢复位置 原对象" }));
    {
        const option = await screen.findByRole("option", { name: "根目录" });
        fireEvent.pointerDown(option, { pointerType: "mouse" });
        fireEvent.click(option);
    }
    h.failWrite();
    fireEvent.click(within(row).getByRole("button", { name: "恢复 原对象" }));
    fireEvent.click(screen.getByRole("button", { name: "确认恢复" }));
    await within(row).findByText(/版本或恢复条件已变化/);
    h.recoverWrite();
    const other = screen.getByRole("region", { name: "回收对象 另一个" });
    fireEvent.click(within(other).getByRole("button", { name: "恢复 另一个" }));
    h.pause();
    fireEvent.click(screen.getByRole("button", { name: "确认恢复" }));
    await screen.findByText("已恢复 另一个");
    expect(screen.getByRole("region", { name: "回收对象 原对象" })).toBe(row);
    expect(row).toHaveTextContent("版本或恢复条件已变化");
    expect(within(row).getByRole("combobox")).toHaveTextContent("根目录");
    await act(async () => h.release());
});
it("写成功后读取失败仍移除已恢复行，并将刷新失败作为读取反馈", async () => {
    const h = harness([item("a", "成功事项", { entity_type: "task" }), item("b", "保留笔记")]);
    h.mount();
    const row = await screen.findByRole("region", { name: "回收对象 成功事项" });
    fireEvent.click(within(row).getByRole("button", { name: "恢复 成功事项" }));
    h.failRead();
    fireEvent.click(screen.getByRole("button", { name: "确认恢复" }));
    await screen.findByText("已恢复 成功事项");
    await screen.findByText(/列表刷新失败/);
    expect(screen.queryByRole("region", { name: "回收对象 成功事项" })).not.toBeInTheDocument();
    expect(screen.getByRole("region", { name: "回收对象 保留笔记" })).toBeInTheDocument();
    expect(screen.queryByText("SQL private")).not.toBeInTheDocument();
    const r = h.requests.find(r => r.method === "POST")!;
    expect(r.url).toContain("/tasks/a/restore");
    expect(await r.json()).toEqual({ expected_version: 7 });
});
it("t42_2 类型筛选只请求trash，归档由领域列表处理", async () => {
    const h = harness([item("a", "笔记"), item("b", "项目", { entity_type: "project" })]);
    h.mount();
    await screen.findByRole("region", { name: "回收对象 笔记" });
    fireEvent.click(screen.getByRole("combobox", { name: "对象类型" }));
    {
        const option = await screen.findByRole("option", { name: "项目" });
        fireEvent.pointerDown(option, { pointerType: "mouse" });
        fireEvent.click(option);
    }
    await waitFor(() => expect(screen.queryByRole("region", { name: "回收对象 笔记" })).not.toBeInTheDocument());
    expect(h.requests.at(-1)?.url).toContain("entity_type=project");
});
it("翻页加载时禁用按钮，类型筛选重置不透明游标", async () => {
    const requests: URL[] = [];
    let release: () => void = () => { };
    const bundle = { queryClient: new QueryClient({ defaultOptions: { queries: { retry: false } } }), api: createApiClient({ baseUrl: "http://localhost/api/v1", fetch: async (input) => {
                const url = new URL((input as Request).url);
                requests.push(url);
                const cursor = url.searchParams.get("cursor");
                if (cursor)
                    await new Promise<void>(resolve => { release = resolve; });
                return new Response(JSON.stringify({ items: [item(cursor ? "b" : "a", cursor ? "第二页" : "第一页")], next_cursor: cursor ? null : "opaque-token" }), { headers: { "content-type": "application/json" } });
            } }) };
    render(<UiProvider><QueryClientProvider client={bundle.queryClient}><TrashPage bundle={bundle}/></QueryClientProvider></UiProvider>);
    await screen.findByRole("region", { name: "回收对象 第一页" });
    fireEvent.click(screen.getByRole("button", { name: "下一页" }));
    await waitFor(() => expect(requests).toHaveLength(2));
    expect(screen.getByRole("button", { name: "下一页" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "上一页" })).toBeDisabled();
    expect(requests[1]!.searchParams.get("cursor")).toBe("opaque-token");
    await act(async () => release());
    await screen.findByRole("region", { name: "回收对象 第二页" });
    fireEvent.click(screen.getByRole("combobox", { name: "对象类型" }));
    const option = await screen.findByRole("option", { name: "事项" });
    fireEvent.pointerDown(option, { pointerType: "mouse" });
    fireEvent.click(option);
    await waitFor(() => expect(requests).toHaveLength(3));
    expect(requests[2]!.searchParams.has("cursor")).toBe(false);
    expect(requests[2]!.searchParams.get("entity_type")).toBe("task");
});
